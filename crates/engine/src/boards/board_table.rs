//! Board tables: a frame of row frames holding cell frames with text, all ordinary auto layout
//! layers. The table hugs its rows' height, the rows share its width and the cells share their
//! row's width, so a cell that wraps more text makes its row taller. Each row but the last draws
//! a line under itself and the column lines are thin rectangles over the table that stretch with
//! it (they are rebuilt after every edit, like the first row's header tint).

use figcraft_color::{Color, Paint};
use figcraft_doc::{
    AutoLayout, BoardItem, Constraints, Document, HConstraint, LayoutMode, Node, NodeId, NodeKind, Sizing, StrokeAlign, TextAlignH, VConstraint,
};
use figcraft_geom::Affine;
use serde_json::{Value, json};

use crate::board::{INK, add_text, require};
use crate::cmd::CommandSpec;
use crate::params::{get, id, num, num_or, opt_index, req_string};
use crate::{EngineError, Result, Session, bad, spec, textedit};

/// The most rows and columns a table has.
pub const MAX_ROWS: usize = 50;
pub const MAX_COLUMNS: usize = 20;
/// A new column's width, and a cell's height before its text is measured.
const CELL_W: f64 = 160.0;
const CELL_H: f64 = 40.0;
/// Room around a cell's text: [top, right, bottom, left].
const CELL_PAD: [f64; 4] = [10.0, 12.0, 10.0, 12.0];
const TEXT_SIZE: f64 = 14.0;
const LINE_W: f64 = 1.0;
const GRID: Color = Color::from_rgb8_const(0xCF, 0xCB, 0xC4);
const HEADER: Color = Color::from_rgb8_const(0xF1, 0xEF, 0xEB);

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "board.table",
            "Table",
            "{rows?, columns? (default 3 × 3, or the size of `cells`; at most 50 × 20), cells? ([[text…]…] row by row), x?, y? (its top-left on the page; default centred in the view)} → {id, rows, columns, cells: [[text id…]…]}",
            true,
            table_cmd
        ),
        spec!(
            "board.tableEdit",
            "Edit table",
            "{id? (a table or a layer in one; default the selection), action: addRow|addColumn|removeRow|removeColumn, index? (where the new one goes, or which one goes; default the last)} → {id, rows, columns, cells}",
            true,
            edit_cmd
        ),
    ]
}

/// Cells copied from a spreadsheet, as `board.table`'s `cells`: the clipboard holds them as text,
/// a row per line and a tab between cells, a cell holding a tab, a line break or a quote quoted
/// (`""` for a quote). At most [`MAX_ROWS`] × [`MAX_COLUMNS`] are kept; the flag says whether
/// there were more. `None` for text without a tab, or with one cell per row (other text: it
/// pastes as text).
pub fn spreadsheet_cells(text: &str) -> Option<(Vec<Vec<String>>, bool)> {
    if !text.contains('\t') {
        return None;
    }
    // A spreadsheet ends its copy with a line break: no empty row after the last.
    let text = text.strip_suffix("\r\n").or_else(|| text.strip_suffix('\n')).unwrap_or(text);
    let (mut rows, mut row, mut cell) = (Vec::new(), Vec::new(), String::new());
    let (mut quoted, mut cell_start) = (false, true);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                '"' => quoted = false,
                c => cell.push(c),
            }
            continue;
        }
        match c {
            '"' if cell_start => quoted = true,
            '\t' => row.push(std::mem::take(&mut cell)),
            '\r' if chars.peek() == Some(&'\n') => continue,
            '\n' | '\r' => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            c => cell.push(c),
        }
        cell_start = matches!(c, '\t' | '\n' | '\r');
    }
    row.push(cell);
    rows.push(row);
    if !rows.iter().any(|r| r.len() > 1) {
        return None;
    }
    let more = rows.len() > MAX_ROWS || rows.iter().any(|r| r.len() > MAX_COLUMNS);
    rows.truncate(MAX_ROWS);
    for r in &mut rows {
        r.truncate(MAX_COLUMNS);
    }
    Some((rows, more))
}

/// A count from 1 to `max` (`default` when missing).
fn count(cmd: &str, p: &Value, key: &str, default: usize, max: usize) -> Result<usize> {
    match num(p, key) {
        None if get(p, key).is_none() => Ok(default),
        Some(v) if v.fract() == 0.0 && (1.0..=max as f64).contains(&v) => Ok(v as usize),
        _ => Err(bad(cmd, format!("`{key}` is a whole number from 1 to {max}"))),
    }
}

/// A frame for the table at `index` in `parent`, made by `f`.
fn frame(doc: &mut Document, parent: NodeId, index: Option<usize>, name: &str, size: (f64, f64), f: impl FnOnce(&mut Node)) -> Result<NodeId> {
    let id = doc.alloc_id();
    let mut n = Node::new(id, NodeKind::Frame, name, size.0, size.1);
    n.fills.clear();
    n.clip = false;
    f(&mut n);
    doc.insert(n, parent, index)?;
    Ok(id)
}

fn make_cell(doc: &mut Document, row: NodeId, index: Option<usize>, width: f64, text: &str) -> Result<NodeId> {
    let cell = frame(doc, row, index, "Cell", (width, CELL_H), |n| {
        n.layout = Some(AutoLayout { mode: LayoutMode::Vertical, gap: 0.0, padding: CELL_PAD, ..AutoLayout::default() });
        n.child_layout.sizing_h = Sizing::Fill;
        n.child_layout.sizing_v = Sizing::Hug;
        n.child_layout.min_h = Some(CELL_H);
    })?;
    add_text(doc, cell, text, TEXT_SIZE, INK, TextAlignH::Left, (width - CELL_PAD[1] - CELL_PAD[3]).max(1.0))?;
    Ok(cell)
}

fn make_row(doc: &mut Document, table: NodeId, index: Option<usize>, width: f64, texts: &[&str]) -> Result<NodeId> {
    let row = frame(doc, table, index, "Row", (width, CELL_H), |n| {
        n.layout = Some(AutoLayout { mode: LayoutMode::Horizontal, gap: 0.0, padding: [0.0; 4], ..AutoLayout::default() });
        n.child_layout.sizing_h = Sizing::Fill;
        n.child_layout.sizing_v = Sizing::Hug;
        n.stroke.weight = LINE_W;
        n.stroke.align = StrokeAlign::Inside;
        n.stroke.sides = Some([0.0, 0.0, LINE_W, 0.0]);
    })?;
    let cw = width / texts.len().max(1) as f64;
    for t in texts {
        make_cell(doc, row, None, cw, t)?;
    }
    Ok(row)
}

/// A table of `texts` (rows of cell texts, all as long) at `at` on `parent`.
pub(crate) fn make_table(doc: &mut Document, parent: NodeId, at: (f64, f64), texts: &[Vec<&str>]) -> Result<NodeId> {
    let columns = texts.first().map_or(1, Vec::len).max(1);
    let w = CELL_W * columns as f64;
    let table = frame(doc, parent, None, "Table", (w, CELL_H * texts.len() as f64), |n| {
        n.transform = Affine::translate(at);
        n.fills = vec![Paint::solid(Color::WHITE)];
        n.strokes = vec![Paint::solid(GRID)];
        n.stroke.weight = LINE_W;
        n.stroke.align = StrokeAlign::Inside;
        n.corners.radii = [4.0; 4];
        n.clip = true;
        // The outline draws over the rows' edges rather than taking room from them.
        n.layout = Some(AutoLayout { mode: LayoutMode::Vertical, gap: 0.0, padding: [0.0; 4], strokes_included: false, ..AutoLayout::default() });
        n.child_layout.sizing_v = Sizing::Hug;
        n.board = Some(BoardItem::Table);
    })?;
    for row in texts {
        make_row(doc, table, None, w, row)?;
    }
    restyle(doc, table)?;
    Ok(table)
}

/// Table `table`'s rows (its laid-out children) and column lines (the others).
fn parts(doc: &Document, table: NodeId) -> (Vec<NodeId>, Vec<NodeId>) {
    let kids = doc.node(table).map(|n| n.children.clone()).unwrap_or_default();
    kids.into_iter().partition(|c| doc.node(*c).is_some_and(|n| !n.child_layout.absolute))
}

fn cells(doc: &Document, row: NodeId) -> Vec<NodeId> {
    doc.node(row).map(|n| n.children.clone()).unwrap_or_default()
}

/// The first row tinted, a line under every row but the last, and the column lines over it all.
fn restyle(doc: &mut Document, table: NodeId) -> Result<()> {
    let (rows, lines) = parts(doc, table);
    for l in lines {
        doc.remove(l)?;
    }
    let last = rows.len().saturating_sub(1);
    for (i, r) in rows.iter().enumerate() {
        let n = doc.get_mut(*r)?;
        n.fills = if i == 0 { vec![Paint::solid(HEADER)] } else { Vec::new() };
        n.strokes = if i < last { vec![Paint::solid(GRID)] } else { Vec::new() };
    }
    let columns = rows.first().map_or(0, |r| cells(doc, *r).len());
    let (w, h) = doc.get(table).map(|t| (t.width, t.height))?;
    for i in 1..columns {
        let x = w * i as f64 / columns as f64 - LINE_W / 2.0;
        let id = doc.alloc_id();
        let mut n = Node::new(id, NodeKind::Rectangle, "Column line", LINE_W, h);
        n.fills = vec![Paint::solid(GRID)];
        n.transform = Affine::translate((x, 0.0));
        n.child_layout.absolute = true;
        n.constraints = Constraints { horizontal: HConstraint::Scale, vertical: VConstraint::TopBottom };
        n.locked = true;
        doc.insert(n, table, None)?;
    }
    Ok(())
}

/// Each cell's text layer, row by row (null for a cell without one).
fn cell_texts(doc: &Document, table: NodeId) -> Value {
    let text = |cell: NodeId| {
        doc.node(cell).and_then(|n| n.children.iter().copied().find(|c| doc.node(*c).is_some_and(|k| matches!(k.kind, NodeKind::Text(_)))))
    };
    let (rows, _) = parts(doc, table);
    Value::Array(rows.iter().map(|r| Value::Array(cells(doc, *r).into_iter().map(|c| json!(text(c))).collect())).collect())
}

fn answer(doc: &Document, table: NodeId) -> Value {
    let (rows, _) = parts(doc, table);
    let columns = rows.first().map_or(0, |r| cells(doc, *r).len());
    json!({"id": table, "rows": rows.len(), "columns": columns, "cells": cell_texts(doc, table)})
}

fn table_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.table";
    require(s, cmd)?;
    let given: Vec<Vec<String>> = match get(p, "cells") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(rows)) => rows
            .iter()
            .map(|r| match r {
                Value::Array(c) => c.iter().map(|t| t.as_str().map(str::to_string).ok_or_else(|| bad(cmd, "`cells` holds text"))).collect(),
                _ => Err(bad(cmd, "`cells` is a list of rows, each a list of texts")),
            })
            .collect::<Result<_>>()?,
        Some(_) => return Err(bad(cmd, "`cells` is a list of rows, each a list of texts")),
    };
    let widest = given.iter().map(Vec::len).max().unwrap_or(0);
    let rows = count(cmd, p, "rows", if given.is_empty() { 3 } else { given.len() }, MAX_ROWS)?;
    let columns = count(cmd, p, "columns", if widest == 0 { 3 } else { widest }, MAX_COLUMNS)?;
    if given.len() > rows || widest > columns {
        return Err(bad(cmd, format!("`cells` doesn't fit in {rows} × {columns}")));
    }
    let texts: Vec<Vec<&str>> =
        (0..rows).map(|r| (0..columns).map(|c| given.get(r).and_then(|row| row.get(c)).map_or("", String::as_str)).collect()).collect();
    let (w, h) = (CELL_W * columns as f64, CELL_H * rows as f64);
    let view = &s.ds()?.view;
    let at = (num_or(p, "x", view.cx - w / 2.0), num_or(p, "y", view.cy - h / 2.0));
    if !(at.0.is_finite() && at.1.is_finite()) {
        return Err(bad(cmd, "`x` and `y` are finite numbers"));
    }
    let page = s.page()?;
    let doc = s.doc_mut()?;
    let table = make_table(doc, page, at, &texts)?;
    let out = answer(doc, table);
    s.set_selection(vec![table]);
    Ok(out)
}

/// The table `id` is or is in.
fn table_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    std::iter::once(id).chain(doc.ancestors(id)).find(|a| doc.node(*a).is_some_and(|n| n.board == Some(BoardItem::Table)))
}

fn edit_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.tableEdit";
    require(s, cmd)?;
    let target = id(p, "id").or_else(|| s.selection().first().copied()).ok_or_else(|| EngineError::Disabled(cmd.into(), "select a table".into()))?;
    let doc = s.doc()?;
    doc.get(target)?;
    let table = table_of(doc, target).ok_or_else(|| bad(cmd, "that isn't a table"))?;
    let action = req_string(cmd, p, "action")?;
    let index = opt_index(cmd, p, "index")?;
    let (rows, _) = parts(doc, table);
    let columns = rows.first().map_or(0, |r| cells(doc, *r).len()).max(1);
    let width = doc.get(table)?.width;
    let doc = s.doc_mut()?;
    // `n` things: `index` (default `last`) must be at most `end`.
    let at = |end: usize, last: usize| match index {
        Some(i) if i > end => Err(bad(cmd, format!("`index` is from 0 to {end}"))),
        Some(i) => Ok(i),
        None => Ok(last),
    };
    match action.as_str() {
        "addRow" => {
            if rows.len() >= MAX_ROWS {
                return Err(bad(cmd, format!("a table has at most {MAX_ROWS} rows")));
            }
            let i = at(rows.len(), rows.len())?;
            // Rows come before the column lines among the table's layers.
            let child = rows.get(i).or(rows.last()).and_then(|r| doc.node(table).and_then(|t| t.children.iter().position(|c| c == r)));
            let child = child.map(|c| if i >= rows.len() { c + 1 } else { c });
            make_row(doc, table, child, width, &vec![""; columns])?;
        }
        "removeRow" => {
            if rows.len() <= 1 {
                return Err(bad(cmd, "a table keeps at least one row"));
            }
            let i = at(rows.len() - 1, rows.len() - 1)?;
            if let Some(r) = rows.get(i) {
                doc.remove(*r)?;
            }
        }
        "addColumn" => {
            if columns >= MAX_COLUMNS {
                return Err(bad(cmd, format!("a table has at most {MAX_COLUMNS} columns")));
            }
            let i = at(columns, columns)?;
            let cw = width / columns as f64;
            doc.get_mut(table)?.width = width + cw;
            for r in &rows {
                make_cell(doc, *r, Some(i), cw, "")?;
            }
        }
        "removeColumn" => {
            if columns <= 1 {
                return Err(bad(cmd, "a table keeps at least one column"));
            }
            let i = at(columns - 1, columns - 1)?;
            doc.get_mut(table)?.width = width - width / columns as f64;
            for r in &rows {
                if let Some(c) = cells(doc, *r).get(i) {
                    doc.remove(*c)?;
                }
            }
        }
        other => return Err(bad(cmd, format!("`action` is addRow, addColumn, removeRow or removeColumn, not `{other}`"))),
    }
    restyle(doc, table)?;
    let out = answer(doc, table);
    s.set_selection(vec![table]);
    Ok(out)
}

/// A double-click on a table: type in the cell under the pointer (`leaf`). False when it hit no
/// cell (a column line, the table's edge).
pub(crate) fn edit_cell(s: &mut Session, doc: &Document, leaf: NodeId) -> Result<bool> {
    let in_table = |c: &NodeId| {
        let row = doc.node(*c).and_then(|n| n.parent);
        row.and_then(|r| doc.node(r)).and_then(|r| r.parent).and_then(|t| doc.node(t)).is_some_and(|t| t.board == Some(BoardItem::Table))
    };
    let Some(cell) = std::iter::once(leaf).chain(doc.ancestors(leaf)).find(in_table) else { return Ok(false) };
    let Some(text) = cells(doc, cell).into_iter().find(|c| doc.node(*c).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)))) else {
        return Ok(false);
    };
    textedit::begin(s, text, None, false)?;
    textedit::key(s, "SelectAll", false, false, false)?;
    Ok(true)
}
