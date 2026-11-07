//! Arrow keys on auto layout children (`transform.nudge`): instead of nudging, a child placed by
//! its auto layout frame moves one place.
//!
//! - **Along the flow** (Left/Right in a row, Up/Down in a column) it swaps with the next placed
//!   sibling; selected neighbours move together. Hidden siblings and those ignoring auto layout
//!   keep their places in the layer order.
//! - **Wrapping rows:** Left/Right move along the flow, Up/Down to the previous/next visual row,
//!   next to the child nearest above/below it.
//! - **Grids:** each arrow moves the child to the neighbouring cell. Two automatically placed
//!   children swap places in the flow; otherwise the child takes the cell (an anchored child there
//!   gets its old cell, automatically placed ones flow around it).
//! - **Across nested auto layout frames:** when the child can't move any further in its own frame
//!   that way (at the end of the flow, across a row's axis, past a grid's edge), and that frame sits
//!   in an auto layout frame flowing that way, the child moves into the neighbouring auto layout
//!   frame there: at its near end when both flow the same way, else at the slot nearest to the
//!   child (a list of rows: Down moves an item into the row below, under where it was).
//!
//! Children ignoring auto layout, and children of frames without it, nudge as usual (the caller
//! handles those).

use figcraft_doc::{AutoLayout, Document, LayoutMode, NodeId};
use figcraft_geom::Point;
use figcraft_layout::{GridCell, flow_children, grid_place};

use crate::Result;
use crate::components::structure_locked;
use crate::drop::{auto_layout_slot, flip, wrap_rows};

/// One arrow key: an axis and a direction (`forward`: right or down).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dir {
    pub horizontal: bool,
    pub forward: bool,
}

impl Dir {
    /// The arrows in a nudge of (`ux`, `uy`) units: horizontal first, then vertical.
    pub fn from_units(ux: f64, uy: f64) -> Vec<Dir> {
        let mut out = Vec::new();
        if ux.is_finite() && ux != 0.0 {
            out.push(Dir { horizontal: true, forward: ux > 0.0 });
        }
        if uy.is_finite() && uy != 0.0 {
            out.push(Dir { horizontal: false, forward: uy > 0.0 });
        }
        out
    }
}

/// The auto layout frame that places `id` (`None` for layers ignoring auto layout, hidden ones and
/// layers of plain frames).
pub fn flowing_parent(doc: &Document, id: NodeId) -> Option<NodeId> {
    let n = doc.node(id)?;
    let parent = n.parent?;
    let pn = doc.node(parent)?;
    (pn.layout.is_some() && pn.kind.is_frame_like() && n.visible && !n.child_layout.absolute).then_some(parent)
}

/// Move the `selected` children of auto layout frame `parent` one place in `dir`. Returns whether
/// anything moved.
pub fn arrow_move(doc: &mut Document, parent: NodeId, selected: &[NodeId], dir: Dir) -> Result<bool> {
    let Some(layout) = doc.node(parent).and_then(|n| n.layout.clone()) else { return Ok(false) };
    let flow = flow_children(doc, parent);
    let block: Vec<NodeId> = flow.iter().copied().filter(|c| selected.contains(c)).collect();
    if block.is_empty() {
        return Ok(false);
    }
    let moved = match layout.mode {
        LayoutMode::Grid => grid_move(doc, parent, &layout, &block, dir)?,
        mode => {
            let along = (mode == LayoutMode::Horizontal) == dir.horizontal;
            if along {
                step_in_flow(doc, parent, &flow, &block, dir.forward)?
            } else if layout.wrap {
                wrap_row_move(doc, parent, &flow, &block, dir.forward, mode == LayoutMode::Vertical)?
            } else {
                false
            }
        }
    };
    if moved {
        return Ok(true);
    }
    move_into_neighbour(doc, parent, &block, dir)
}

/// Write a new order of `parent`'s placed children back: the slots placed children held in the
/// child list take `flow` in order; the other children keep theirs.
fn write_flow(doc: &mut Document, parent: NodeId, flow: &[NodeId]) -> Result<()> {
    let p = doc.get_mut(parent)?;
    let mut next = flow.iter().copied();
    let placed: Vec<NodeId> = flow.to_vec();
    for c in p.children.iter_mut() {
        if placed.contains(c)
            && let Some(n) = next.next()
        {
            *c = n;
        }
    }
    Ok(())
}

/// Swap each selected child with the next unselected placed sibling that way (selected neighbours
/// move as a block). Returns whether anything moved.
fn step_in_flow(doc: &mut Document, parent: NodeId, flow: &[NodeId], block: &[NodeId], forward: bool) -> Result<bool> {
    let mut order = flow.to_vec();
    let idx: Vec<usize> = if forward { (0..order.len()).rev().collect() } else { (0..order.len()).collect() };
    for i in idx {
        let Some(j) = (if forward { i.checked_add(1) } else { i.checked_sub(1) }) else { continue };
        let (Some(a), Some(b)) = (order.get(i).copied(), order.get(j).copied()) else { continue };
        if block.contains(&a) && !block.contains(&b) {
            order.swap(i, j);
        }
    }
    if order == flow {
        return Ok(false);
    }
    write_flow(doc, parent, &order)?;
    Ok(true)
}

/// Up/Down in a wrapping row layout (Left/Right in a wrapping column layout, `vertical`): into the
/// previous/next visual row, next to the child nearest the selection's centre (before it going
/// back, after it going forward, so the selection lands beside where it was once the rows reflow).
/// Returns whether anything moved.
fn wrap_row_move(doc: &mut Document, parent: NodeId, flow: &[NodeId], block: &[NodeId], forward: bool, vertical: bool) -> Result<bool> {
    let rows = wrap_rows(doc, flow, vertical);
    let lead = if forward { block.last() } else { block.first() };
    let Some(lead_i) = lead.and_then(|l| flow.iter().position(|c| c == l)) else { return Ok(false) };
    let Some(row) = rows.iter().position(|r| r.iter().any(|(i, _)| *i == lead_i)) else { return Ok(false) };
    let target_row = if forward { row.checked_add(1) } else { row.checked_sub(1) };
    let Some(target) = target_row.and_then(|r| rows.get(r)) else { return Ok(false) };
    let Some(centre) = doc.bounds_of(block).map(|b| flip(b, vertical).center().x) else { return Ok(false) };
    let nearest = target
        .iter()
        .filter(|(i, _)| flow.get(*i).is_some_and(|c| !block.contains(c)))
        .min_by(|a, b| (a.1.center().x - centre).abs().total_cmp(&(b.1.center().x - centre).abs()))
        .and_then(|(i, _)| flow.get(*i).copied());
    let Some(nearest) = nearest else { return Ok(false) };
    let mut order: Vec<NodeId> = flow.iter().copied().filter(|c| !block.contains(c)).collect();
    let Some(at) = order.iter().position(|c| *c == nearest) else { return Ok(false) };
    let at = if forward { at + 1 } else { at };
    for (k, c) in block.iter().enumerate() {
        order.insert((at + k).min(order.len()), *c);
    }
    write_flow(doc, parent, &order)?;
    Ok(true)
}

/// Whether grid cell `c` covers column `col`, row `row`.
fn covers(c: &GridCell, col: usize, row: usize) -> bool {
    (c.column..c.column + c.column_span).contains(&col) && (c.row..c.row + c.row_span).contains(&row)
}

/// An arrow in a grid: each selected child moves to the neighbouring cell. Returns whether anything
/// moved (`false` at the grid's edge).
fn grid_move(doc: &mut Document, parent: NodeId, layout: &AutoLayout, block: &[NodeId], dir: Dir) -> Result<bool> {
    let cols = (layout.grid_columns as usize).max(1);
    let mut moved = false;
    // The leading child first, so the ones behind it follow into the cells it frees.
    let order: Vec<NodeId> = if dir.forward { block.iter().rev().copied().collect() } else { block.to_vec() };
    for id in order {
        let flow = flow_children(doc, parent);
        let (cells, rows) = grid_place(doc, layout, &flow);
        let Some(cell) = cells.iter().find(|c| c.id == id).copied() else { continue };
        let (dc, dr): (isize, isize) = match (dir.horizontal, dir.forward) {
            (true, true) => (1, 0),
            (true, false) => (-1, 0),
            (false, true) => (0, 1),
            (false, false) => (0, -1),
        };
        let (Some(tc), Some(tr)) = (cell.column.checked_add_signed(dc), cell.row.checked_add_signed(dr)) else { continue };
        if tc + cell.column_span > cols || tr + cell.row_span > rows {
            continue;
        }
        // The cell the child moves into first.
        let lead = (if dc > 0 { tc + cell.column_span - 1 } else { tc }, if dr > 0 { tr + cell.row_span - 1 } else { tr });
        let occupant = cells.iter().find(|c| c.id != id && covers(c, lead.0, lead.1)).copied();
        let anchored =
            |doc: &Document, i: NodeId| doc.node(i).is_some_and(|n| n.child_layout.grid_column.is_some() || n.child_layout.grid_row.is_some());
        match occupant {
            Some(o) if !anchored(doc, id) && !anchored(doc, o.id) => {
                // Both flow automatically: swap places in the flow.
                let mut order: Vec<NodeId> = flow.iter().copied().filter(|c| *c != id).collect();
                let Some(at) = order.iter().position(|c| *c == o.id) else { continue };
                order.insert(if dir.forward { at + 1 } else { at }, id);
                write_flow(doc, parent, &order)?;
            }
            _ => {
                let to_u32 = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
                if let Some(o) = occupant.filter(|o| anchored(doc, o.id)) {
                    let cl = &mut doc.get_mut(o.id)?.child_layout;
                    cl.grid_column = Some(to_u32(cell.column));
                    cl.grid_row = Some(to_u32(cell.row));
                }
                let cl = &mut doc.get_mut(id)?.child_layout;
                cl.grid_column = Some(to_u32(tc));
                cl.grid_row = Some(to_u32(tr));
            }
        }
        moved = true;
    }
    Ok(moved)
}

/// Move `block` (children of `parent`) into the auto layout frame next to `parent` in `dir`, when
/// `parent` is placed by an auto layout frame flowing that way. Returns whether anything moved.
fn move_into_neighbour(doc: &mut Document, parent: NodeId, block: &[NodeId], dir: Dir) -> Result<bool> {
    let Some(outer) = flowing_parent(doc, parent) else { return Ok(false) };
    let Some(outer_layout) = doc.node(outer).and_then(|n| n.layout.clone()) else { return Ok(false) };
    let flows_that_way = match outer_layout.mode {
        LayoutMode::Horizontal => dir.horizontal,
        LayoutMode::Vertical => !dir.horizontal,
        LayoutMode::Grid => false,
    };
    if !flows_that_way {
        return Ok(false);
    }
    let siblings = flow_children(doc, outer);
    let Some(i) = siblings.iter().position(|c| *c == parent) else { return Ok(false) };
    let next = if dir.forward { i.checked_add(1) } else { i.checked_sub(1) };
    let Some(dest) = next.and_then(|j| siblings.get(j)).copied() else { return Ok(false) };
    let accepts =
        doc.node(dest).is_some_and(|n| n.kind.is_frame_like() && !n.locked && n.layout.as_ref().is_some_and(|l| l.mode != LayoutMode::Grid))
            && !structure_locked(doc, dest);
    if !accepts {
        return Ok(false);
    }
    let (Some(b), Some(sel)) = (doc.world_bounds(dest), doc.bounds_of(block)) else { return Ok(false) };
    // Enter at the near edge, level with the selection on the other axis (just before its centre,
    // so a child level with it is displaced and the selection keeps its place).
    let edge = |lo: f64, hi: f64| if dir.forward { lo + (hi - lo).min(0.5) } else { hi - (hi - lo).min(0.5) };
    let level = |c: f64, lo: f64, hi: f64| (c - 0.5).clamp(lo, hi.max(lo));
    let probe = if dir.horizontal {
        Point::new(edge(b.x0, b.x1), level(sel.center().y, b.y0, b.y1))
    } else {
        Point::new(level(sel.center().x, b.x0, b.x1), edge(b.y0, b.y1))
    };
    let Some((index, _)) = auto_layout_slot(doc, dest, probe, block) else { return Ok(false) };
    for (k, c) in block.iter().enumerate() {
        doc.reparent(*c, dest, Some(index + k))?;
    }
    Ok(true)
}
