//! Geometry for reordering a smart selection. Only regular, non-overlapping rows, columns
//! and complete top-left-aligned grids have unambiguous insertion slots.
//! Marking several items and irregular grids are out of scope.
use figcraft_doc::{Document, NodeId};
use figcraft_geom::{Point, Rect};

use crate::selection_spacing::selection_spacing;

#[derive(Clone, Debug)]
pub struct ReorderLayout {
    /// Visual order, independent of selection order and layer stacking.
    pub slots: Vec<(NodeId, Rect)>,
    columns: usize,
    rows: usize,
    gap_x: f64,
    gap_y: f64,
}

impl ReorderLayout {
    pub fn new(doc: &Document, ids: &[NodeId]) -> Option<Self> {
        let layout = selection_spacing(doc, ids);
        let h = layout.horizontal.as_ref();
        let v = layout.vertical.as_ref();
        let gap = |axis: Option<&crate::selection_spacing::Axis>| match axis {
            Some(a) => a.value.filter(|v| *v >= 0.0),
            None => Some(0.0),
        };
        let (gap_x, gap_y) = (gap(h)?, gap(v)?);
        let columns = h.map_or(1, |a| a.tracks.len());
        let rows = v.map_or(1, |a| a.tracks.len());
        if ids.len() < 2 || columns.checked_mul(rows)? != ids.len() {
            return None;
        }
        let mut cells = std::collections::BTreeMap::new();
        for id in ids {
            let col = match h {
                Some(a) => a.tracks.iter().position(|t| t.ids.contains(id))?,
                None => 0,
            };
            let row = match v {
                Some(a) => a.tracks.iter().position(|t| t.ids.contains(id))?,
                None => 0,
            };
            let b = doc.world_bounds(*id)?;
            if columns > 1 && rows > 1 {
                let x = h?.tracks.get(col)?.bounds.x0;
                let y = v?.tracks.get(row)?.bounds.y0;
                if (b.x0 - x).abs() > 0.01 || (b.y0 - y).abs() > 0.01 {
                    return None;
                }
            }
            if cells.insert((row, col), (*id, b)).is_some() {
                return None;
            }
        }
        Some(Self { slots: cells.into_values().collect(), columns, rows, gap_x, gap_y })
    }

    /// Insertion shifts intervening items; swapping exchanges just two entries. Sizes and
    /// original gaps are retained, so unequal-sized layers reflow instead of overlapping.
    pub fn positions(&self, source: NodeId, target: NodeId, swap: bool) -> Option<Vec<(NodeId, Point)>> {
        let from = self.slots.iter().position(|(id, _)| *id == source)?;
        let to = self.slots.iter().position(|(id, _)| *id == target)?;
        if from == to {
            return Some(self.slots.iter().map(|(id, b)| (*id, Point::new(b.x0, b.y0))).collect());
        }
        let mut order = self.slots.clone();
        if swap {
            order.swap(from, to);
        } else {
            let item = order.remove(from);
            order.insert(to, item);
        }
        let mut widths = vec![0.0_f64; self.columns];
        let mut heights = vec![0.0_f64; self.rows];
        for (i, (_, b)) in order.iter().enumerate() {
            let w = widths.get_mut(i % self.columns)?;
            *w = w.max(b.width());
            let h = heights.get_mut(i / self.columns)?;
            *h = h.max(b.height());
        }
        let x0 = self.slots.iter().map(|(_, b)| b.x0).fold(f64::INFINITY, f64::min);
        let y0 = self.slots.iter().map(|(_, b)| b.y0).fold(f64::INFINITY, f64::min);
        let starts = |sizes: &[f64], mut at: f64, gap: f64| {
            sizes
                .iter()
                .map(|size| {
                    let start = at;
                    at += size + gap;
                    start
                })
                .collect::<Vec<_>>()
        };
        let xs = starts(&widths, x0, self.gap_x);
        let ys = starts(&heights, y0, self.gap_y);
        order
            .iter()
            .enumerate()
            .map(|(i, (id, b))| {
                let x = if self.columns == 1 { b.x0 } else { *xs.get(i % self.columns)? };
                let y = if self.rows == 1 { b.y0 } else { *ys.get(i / self.columns)? };
                Some((*id, Point::new(x, y)))
            })
            .collect()
    }
}
