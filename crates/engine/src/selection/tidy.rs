//! Tidy up: the moves that even a selection's gaps the way Figma's web app does.
//!
//! Measured in Figma's web app on 2026-11-12 (top-level rectangles): a row, a column and a 2 x 3
//! grid of unequal sizes, and eight runs on the gap. Figma keeps the order and the sizes of every
//! layer and keeps the first layer where it is. A row evens its horizontal gaps and keeps every Y,
//! a column the mirror image. The one gap is the most common existing gap, else the mean of the
//! gaps taken down to a whole number. A grid is tidied by columns: each column is as wide as its
//! widest layer, centred where its layers were, the columns are evened, every layer is centred in
//! its column and every column is stacked from the selection's top. Tidy up is greyed out for fewer
//! than three layers and for layers that overlap.
//!
//! Not measured, FigCraft's choices: the smaller of two equally common gaps; a mean ending in .5
//! going down; gaps a pixel apart being different gaps; a diagonal being a row; a grid with an
//! empty place; a grid always read by columns; one overlapping pair among other layers making it
//! unavailable; layers in an auto layout flow making it unavailable; the 2,000 layer cap; rotated
//! layers counting by their box; an explicit `spacing` taking two layers; no rounding in a grid
//! with Snap to pixel grid off. A selection that is none of a row, a column or a grid keeps the
//! earlier rule (the cells of the largest layer).

use std::collections::BTreeSet;

use figcraft_doc::{Document, NodeId};
use figcraft_geom::Rect;

use crate::cmd::edit::roots;

const EPS: f64 = 0.01;
const MAX_LAYERS: usize = 2000;

type Item = (NodeId, Rect);
type Move = (NodeId, f64, f64);

/// Whether Tidy up is offered for `ids`: three to 2,000 layers with boxes, none in an auto layout
/// flow, none overlapping another.
pub fn can_tidy(doc: &Document, ids: &[NodeId]) -> bool {
    available(doc, ids, 3).is_some()
}

/// The page-space moves that tidy `ids` (empty when unavailable or when nothing would move).
/// `spacing` sets the gap and lets two layers through; `snap` rounds a grid's column positions to
/// whole pixels.
pub(crate) fn tidy_moves(doc: &Document, ids: &[NodeId], spacing: Option<f64>, snap: bool) -> Vec<Move> {
    let min = if spacing.is_some() { 2 } else { 3 };
    let Some(items) = available(doc, ids, min) else { return Vec::new() };
    let cols = groups(&items, false);
    let rows = groups(&items, true);
    let moves = if cols.len() == items.len() {
        line_moves(&items, false, spacing)
    } else if rows.len() == items.len() {
        line_moves(&items, true, spacing)
    } else if is_grid(&cols, &rows) {
        grid_moves(&cols, spacing, snap)
    } else {
        irregular_moves(items, spacing)
    };
    moves.into_iter().filter(|(_, dx, dy)| !(dx.is_finite() && dy.is_finite()) || dx.abs() > 1e-9 || dy.abs() > 1e-9).collect()
}

fn available(doc: &Document, ids: &[NodeId], min: usize) -> Option<Vec<Item>> {
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
    let overlapping = items.iter().enumerate().any(|(i, a)| items.iter().skip(i + 1).any(|b| overlap(a.1, b.1)));
    if overlapping { None } else { Some(items) }
}

/// Two boxes overlap when they share more than `EPS` on both axes; touching edges do not count.
fn overlap(a: Rect, b: Rect) -> bool {
    a.x1.min(b.x1) - a.x0.max(b.x0) > EPS && a.y1.min(b.y1) - a.y0.max(b.y0) > EPS
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

/// No two layers share both a column and a row.
fn is_grid(cols: &[Vec<Item>], rows: &[Vec<Item>]) -> bool {
    cols.iter().all(|c| rows.iter().all(|r| c.iter().filter(|a| r.iter().any(|b| b.0 == a.0)).count() <= 1))
}

/// The one gap an axis is evened to: `spacing`, a single gap, the most common gap, else the mean
/// taken down to a whole number.
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
        _ => (gaps.iter().sum::<f64>() / gaps.len() as f64 + 1e-9).floor(),
    }
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
    let gap = even_gap(&gaps, spacing);
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

/// A column box: where it starts and how wide it is (its widest layer, around the middle of the
/// column's X range).
fn column_box(col: &[Item]) -> (f64, f64) {
    let lo = col.iter().map(|i| i.1.x0).fold(f64::MAX, f64::min);
    let hi = col.iter().map(|i| i.1.x1).fold(f64::MIN, f64::max);
    let w = col.iter().map(|i| i.1.width()).fold(0.0, f64::max);
    ((lo + hi) / 2.0 - w / 2.0, w)
}

/// Vertical gaps inside every column, pooled.
fn column_gaps(cols: &[Vec<Item>]) -> Vec<f64> {
    let mut out = Vec::new();
    for col in cols {
        let mut c = col.clone();
        c.sort_by(|a, b| a.1.y0.total_cmp(&b.1.y0));
        out.extend(c.windows(2).filter_map(|w| match w {
            [a, b] => Some(b.1.y0 - a.1.y1),
            _ => None,
        }));
    }
    out
}

fn grid_moves(cols: &[Vec<Item>], spacing: Option<f64>, snap: bool) -> Vec<Move> {
    let boxes: Vec<(f64, f64)> = cols.iter().map(|c| column_box(c)).collect();
    let across: Vec<f64> = boxes
        .windows(2)
        .filter_map(|w| match w {
            [a, b] => Some(b.0 - (a.0 + a.1)),
            _ => None,
        })
        .collect();
    let gx = even_gap(&across, spacing);
    let gy = even_gap(&column_gaps(cols), spacing);
    let top = cols.iter().flatten().map(|i| i.1.y0).fold(f64::MAX, f64::min);
    let mut left = boxes.first().map_or(0.0, |b| snap_to(b.0, snap));
    let mut out = Vec::new();
    for (col, (_, w)) in cols.iter().zip(&boxes) {
        let mut c = col.clone();
        c.sort_by(|a, b| a.1.y0.total_cmp(&b.1.y0));
        let mut y = top;
        for (id, b) in c {
            let x = snap_to(left + (w - b.width()) / 2.0, snap);
            out.push((id, x - b.x0, y - b.y0));
            y += b.height() + gy;
        }
        left += w + gx;
    }
    out
}

/// The earlier rule, unchanged: rows by overlapping vertical centres on a grid whose every cell is
/// the largest layer's size, from the union's top left, at the average horizontal gap (at least 10).
fn irregular_moves(mut items: Vec<Item>, spacing: Option<f64>) -> Vec<Move> {
    items.sort_by(|a, b| a.1.y0.total_cmp(&b.1.y0));
    let mut rows: Vec<Vec<Item>> = Vec::new();
    for it in items {
        match rows.last_mut() {
            Some(row) if row.iter().any(|(_, b)| it.1.center().y < b.y1 && it.1.center().y > b.y0) => row.push(it),
            _ => rows.push(vec![it]),
        }
    }
    let all: Vec<Rect> = rows.iter().flatten().map(|(_, b)| *b).collect();
    let (Some(origin), maxw, maxh) = (
        all.iter().copied().reduce(|a, b| a.union(b)),
        all.iter().map(Rect::width).fold(0.0, f64::max),
        all.iter().map(Rect::height).fold(0.0, f64::max),
    ) else {
        return Vec::new();
    };
    let gap = spacing.unwrap_or_else(|| average_gap(&rows));
    let mut out = Vec::new();
    for (ri, row) in rows.iter_mut().enumerate() {
        row.sort_by(|a, b| a.1.x0.total_cmp(&b.1.x0));
        for (ci, (id, b)) in row.iter().enumerate() {
            let x = origin.x0 + ci as f64 * (maxw + gap);
            let y = origin.y0 + ri as f64 * (maxh + gap);
            out.push((*id, x - b.x0, y - b.y0));
        }
    }
    out
}

/// The average existing horizontal gap of the widest row, at least 10.
fn average_gap(rows: &[Vec<Item>]) -> f64 {
    10f64.max(
        rows.iter()
            .filter(|r| r.len() > 1)
            .map(|r| {
                (r.iter().map(|(_, b)| b.x1).fold(f64::MIN, f64::max)
                    - r.iter().map(|(_, b)| b.x0).fold(f64::MAX, f64::min)
                    - r.iter().map(|(_, b)| b.width()).sum::<f64>())
                    / (r.len() - 1) as f64
            })
            .fold(0.0, f64::max),
    )
}
