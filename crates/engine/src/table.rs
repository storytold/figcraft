//! Tables on boards: a frame of cells in rows and columns, each cell a rectangle with text.
//!
//! The frame is marked `craftjam.table` (its column widths, comma separated) and each cell
//! `craftjam.cell` ("row,column"). After every edit a touched table lays its cells out again
//! ([`follow`]): columns as wide as they're set, each row as tall as its tallest cell's words need.

use std::collections::BTreeSet;

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Node, NodeId, NodeKind, shape_text};
use figcraft_geom::Affine;

pub const TABLE: &str = "craftjam.table";
pub const CELL: &str = "craftjam.cell";

pub const COL_W: f64 = 140.0;
pub const ROW_H: f64 = 44.0;
const PAD: f64 = 12.0;
pub const MAX: usize = 100;

const LINE: Color = Color::from_rgb8_const(0xC9, 0xC9, 0xC9);
const HEADER: Color = Color::from_rgb8_const(0xF0, 0xF0, 0xF0);

pub fn is_table(n: &Node) -> bool {
    n.kind.is_frame_like() && n.plugin_data.contains_key(TABLE)
}

/// A cell's (row, column).
pub fn cell_of(n: &Node) -> Option<(usize, usize)> {
    let v = n.plugin_data.get(CELL)?;
    let (r, c) = v.split_once(',')?;
    Some((r.trim().parse().ok()?, c.trim().parse().ok()?))
}

/// The table's column widths.
pub fn widths(n: &Node) -> Vec<f64> {
    n.plugin_data
        .get(TABLE)
        .map(|v| v.split(',').filter_map(|w| w.trim().parse::<f64>().ok()).filter(|w| w.is_finite()).map(|w| w.clamp(24.0, 2000.0)).collect())
        .unwrap_or_default()
}

fn set_widths(n: &mut Node, w: &[f64]) {
    n.plugin_data.insert(TABLE.into(), w.iter().map(|x| format!("{x:.1}")).collect::<Vec<_>>().join(","));
}

/// The table's cells: (row, column) → cell.
pub fn cells(doc: &Document, table: NodeId) -> Vec<((usize, usize), NodeId)> {
    let mut out: Vec<((usize, usize), NodeId)> = doc
        .node(table)
        .map(|t| t.children.iter().filter_map(|c| doc.node(*c)).filter_map(|c| cell_of(c).map(|rc| (rc, c.id))).collect())
        .unwrap_or_default();
    out.sort();
    out
}

/// How many rows the table has.
pub fn rows(doc: &Document, table: NodeId) -> usize {
    cells(doc, table).iter().map(|((r, _), _)| r + 1).max().unwrap_or(0)
}

fn new_cell(id: NodeId, r: usize, c: usize, w: f64) -> Node {
    let mut n = Node::new(id, NodeKind::Rectangle, format!("Cell {}{}", (b'A' + (c % 26) as u8) as char, r + 1), w, ROW_H);
    n.fills = vec![Paint::solid(if r == 0 { HEADER } else { Color::WHITE })];
    n.strokes = vec![Paint::solid(LINE)];
    n.stroke.weight = 1.0;
    n.stroke.align = figcraft_doc::StrokeAlign::Center;
    n.plugin_data.insert(CELL.into(), format!("{r},{c}"));
    n
}

/// A new `rows`×`cols` table `id` with its top left at (`x`, `y`) in its parent's space (the
/// cells are made by [`fill`]).
pub fn new_table(id: NodeId, x: f64, y: f64, cols: usize) -> Node {
    let mut t = Node::new(id, NodeKind::Frame, "Table", COL_W * cols as f64, ROW_H);
    t.fills.clear();
    t.clip = false;
    t.transform = Affine::translate((x, y));
    set_widths(&mut t, &vec![COL_W; cols]);
    t
}

/// Make the cells a table lacks for `rows` rows and its columns.
pub fn fill(doc: &mut Document, table: NodeId, rows: usize) {
    let cols = doc.node(table).map(widths).unwrap_or_default();
    let have: BTreeSet<(usize, usize)> = cells(doc, table).into_iter().map(|(rc, _)| rc).collect();
    for r in 0..rows.min(MAX) {
        for (c, w) in cols.iter().enumerate() {
            if !have.contains(&(r, c)) {
                let id = doc.alloc_id();
                let _ = doc.insert(new_cell(id, r, c, *w), table, None);
            }
        }
    }
}

/// Insert a row before row `at` (a row past the end: at the end).
pub fn insert_row(doc: &mut Document, table: NodeId, at: usize) {
    let n = rows(doc, table);
    if n >= MAX {
        return;
    }
    for ((r, c), id) in cells(doc, table) {
        if r >= at
            && let Some(cell) = doc.node_mut(id)
        {
            cell.plugin_data.insert(CELL.into(), format!("{},{c}", r + 1));
        }
    }
    fill(doc, table, n + 1);
}

/// Insert a column before column `at`.
pub fn insert_column(doc: &mut Document, table: NodeId, at: usize) {
    let Some(t) = doc.node(table) else { return };
    let mut w = widths(t);
    if w.len() >= MAX {
        return;
    }
    let at = at.min(w.len());
    w.insert(at, COL_W);
    for ((r, c), id) in cells(doc, table) {
        if c >= at
            && let Some(cell) = doc.node_mut(id)
        {
            cell.plugin_data.insert(CELL.into(), format!("{r},{}", c + 1));
        }
    }
    if let Some(t) = doc.node_mut(table) {
        set_widths(t, &w);
    }
    let n = rows(doc, table);
    fill(doc, table, n);
}

/// Delete row `at` (a table keeps at least one).
pub fn delete_row(doc: &mut Document, table: NodeId, at: usize) {
    if rows(doc, table) <= 1 {
        return;
    }
    for ((r, c), id) in cells(doc, table) {
        if r == at {
            let _ = doc.remove(id);
        } else if r > at
            && let Some(cell) = doc.node_mut(id)
        {
            cell.plugin_data.insert(CELL.into(), format!("{},{c}", r - 1));
        }
    }
}

/// Delete column `at` (a table keeps at least one).
pub fn delete_column(doc: &mut Document, table: NodeId, at: usize) {
    let Some(t) = doc.node(table) else { return };
    let mut w = widths(t);
    if w.len() <= 1 || at >= w.len() {
        return;
    }
    w.remove(at);
    for ((r, c), id) in cells(doc, table) {
        if c == at {
            let _ = doc.remove(id);
        } else if c > at
            && let Some(cell) = doc.node_mut(id)
        {
            cell.plugin_data.insert(CELL.into(), format!("{r},{}", c - 1));
        }
    }
    if let Some(t) = doc.node_mut(table) {
        set_widths(t, &w);
    }
}

/// Lay out every table in `touched` (or with a touched cell or cell text): columns at their
/// widths, rows as tall as their tallest cell's words. Returns the layers changed.
pub fn follow(doc: &mut Document, touched: &BTreeSet<NodeId>) -> Vec<NodeId> {
    let mut tables = BTreeSet::new();
    for t in touched {
        let mut at = Some(*t);
        // A cell's text → its cell → its table.
        for _ in 0..3 {
            let Some(id) = at else { break };
            let Some(n) = doc.node(id) else { break };
            if is_table(n) {
                tables.insert(id);
                break;
            }
            at = n.text_host.or(n.parent);
        }
    }
    if tables.is_empty() {
        return Vec::new();
    }
    let before: BTreeSet<NodeId> = doc.dirty().iter().copied().collect();
    for t in tables {
        let Some(w) = doc.node(t).map(widths) else { continue };
        let list = cells(doc, t);
        let rows = list.iter().map(|((r, _), _)| r + 1).max().unwrap_or(0);
        // Each row as tall as its tallest cell's words need.
        let mut heights = vec![ROW_H; rows];
        for ((r, c), id) in &list {
            let width = w.get(*c).copied().unwrap_or(COL_W);
            if let Some(text) = shape_text::text_of(doc, *id).and_then(|x| doc.node(x)) {
                let mut probe = text.clone();
                probe.width = (width - 2.0 * PAD).max(1.0);
                if let NodeKind::Text(d) = &mut probe.kind {
                    d.auto_resize = figcraft_doc::TextAutoResize::Height;
                }
                if let (Some((_, th)), Some(h)) = (figcraft_text::auto_size(&probe), heights.get_mut(*r)) {
                    *h = h.max(th + 2.0 * PAD);
                }
            }
        }
        let xs: Vec<f64> = w
            .iter()
            .scan(0.0, |acc, x| {
                let at = *acc;
                *acc += x;
                Some(at)
            })
            .collect();
        let ys: Vec<f64> = heights
            .iter()
            .scan(0.0, |acc, h| {
                let at = *acc;
                *acc += h;
                Some(at)
            })
            .collect();
        for ((r, c), id) in &list {
            let (Some(x), Some(y), Some(cw), Some(rh)) = (xs.get(*c), ys.get(*r), w.get(*c), heights.get(*r)) else { continue };
            if let Some(cell) = doc.node_mut(*id) {
                let tr = Affine::translate((*x, *y));
                if cell.transform != tr || (cell.width - cw).abs() > 1e-6 || (cell.height - rh).abs() > 1e-6 {
                    cell.transform = tr;
                    cell.width = *cw;
                    cell.height = *rh;
                }
            }
        }
        let (tw, th) = (w.iter().sum::<f64>(), heights.iter().sum::<f64>());
        if let Some(tn) = doc.node_mut(t)
            && ((tn.width - tw).abs() > 1e-6 || (tn.height - th).abs() > 1e-6)
        {
            tn.width = tw;
            tn.height = th;
        }
    }
    doc.dirty().iter().copied().filter(|i| !before.contains(i)).collect()
}
