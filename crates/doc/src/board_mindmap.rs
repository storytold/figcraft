//! Mind map layout: where the nodes of a map go, from their sizes and their tree. Pure geometry,
//! like `board_route`; the engine's normalize pass reads the tree from the branches and moves the
//! layers (#2226).

use figcraft_geom::{Point, Rect};

use crate::NodeId;

/// The room between a node's column and its children's column.
pub const MIND_GAP_X: f64 = 88.0;
/// The room between a node (with everything under it) and the next one below.
pub const MIND_GAP_Y: f64 = 48.0;
/// The most nodes one map is laid out with (a larger one is left as it is).
pub const MIND_MAX_NODES: usize = 2000;
/// The deepest level of a map that is laid out (the first node is level 0).
pub const MIND_MAX_DEPTH: usize = 64;

/// One node of a mind map as the layout sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MindBox {
    pub id: NodeId,
    /// Its box on the page now. The root's stays; of the others only the size counts, and the
    /// centre's y, which orders siblings.
    pub rect: Rect,
    /// Its parent's index in the slice (`None`: the root, entry 0). Always a smaller index.
    pub parent: Option<usize>,
    /// Left of the root. Read on the root's children only: deeper nodes follow their parent.
    pub left: bool,
}

fn finite(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite())
}

/// Where every node but the root goes: its box's top-left on the page. Nodes whose parent isn't
/// an earlier entry, that are deeper than [`MIND_MAX_DEPTH`] or that would land off the finite
/// plane are left out; a slice that is empty, longer than [`MIND_MAX_NODES`], whose first entry
/// has a parent or that holds a box that is not finite is left alone altogether.
pub fn mind_layout(nodes: &[MindBox]) -> Vec<(NodeId, Point)> {
    let n = nodes.len();
    let Some(root) = nodes.first().filter(|r| r.parent.is_none()) else { return Vec::new() };
    if n > MIND_MAX_NODES || !nodes.iter().all(|b| finite(b.rect)) {
        return Vec::new();
    }
    // The tree: who is a child of whom, which side of the root it is on, how deep.
    let (mut left, mut depth, mut valid) = (vec![false; n], vec![0_usize; n], vec![false; n]);
    if let Some(v) = valid.first_mut() {
        *v = true;
    }
    // Children by parent and side (0: right, 1: left; below the root only the parent's side is used).
    let mut kids: Vec<[Vec<usize>; 2]> = vec![[Vec::new(), Vec::new()]; n];
    for (i, b) in nodes.iter().enumerate().skip(1) {
        let Some(p) = b.parent.filter(|p| *p < i && valid.get(*p).copied().unwrap_or(false)) else { continue };
        let d = depth.get(p).copied().unwrap_or(0) + 1;
        if d > MIND_MAX_DEPTH {
            continue;
        }
        let side = if p == 0 { b.left } else { left.get(p).copied().unwrap_or(false) };
        if let (Some(v), Some(l), Some(dp)) = (valid.get_mut(i), left.get_mut(i), depth.get_mut(i)) {
            (*v, *l, *dp) = (true, side, d);
        }
        if let Some(k) = kids.get_mut(p) {
            k[usize::from(side)].push(i);
        }
    }
    // Siblings keep the order they are in: by their middles, then by id.
    for k in kids.iter_mut().flatten() {
        k.sort_by(|a, b| {
            let (x, y) = (nodes.get(*a), nodes.get(*b));
            match (x, y) {
                (Some(x), Some(y)) => x.rect.center().y.total_cmp(&y.rect.center().y).then(x.id.cmp(&y.id)),
                _ => std::cmp::Ordering::Equal,
            }
        });
    }
    let height = |i: usize| nodes.get(i).map_or(0.0, |b| b.rect.height());
    let width = |i: usize| nodes.get(i).map_or(0.0, |b| b.rect.width());
    // Span, backward: a node's room is its own height or its children's stack, whichever is more.
    let stack = |span: &[f64], list: &[usize]| {
        let total: f64 = list.iter().map(|c| span.get(*c).copied().unwrap_or(0.0)).sum();
        total + MIND_GAP_Y * list.len().saturating_sub(1) as f64
    };
    let mut span = vec![0.0_f64; n];
    for i in (1..n).rev() {
        let own = kids.get(i).map_or(0.0, |k| stack(&span, &k[usize::from(left.get(i).copied().unwrap_or(false))]));
        if let Some(s) = span.get_mut(i) {
            *s = height(i).max(own);
        }
    }
    // Place, forward. `column[i]`: where node i's children's column begins (its left edge on the
    // right side, its right edge on the left side); `centre[i]`: its middle.
    let mut column = vec![0.0_f64; n];
    let mut centre = vec![0.0_f64; n];
    let mut out = Vec::new();
    let root_c = root.rect.center().y;
    if let Some(c) = centre.first_mut() {
        *c = root_c;
    }
    let mut ok = vec![false; n];
    if let Some(o) = ok.first_mut() {
        *o = true;
    }
    for i in 0..n {
        if !ok.get(i).copied().unwrap_or(false) {
            continue;
        }
        let Some(k) = kids.get(i) else { continue };
        for side in [false, true] {
            let list = k[usize::from(side)].as_slice();
            if list.is_empty() {
                continue;
            }
            // The column the group starts in: past the widest node of its parent's own group.
            let x =
                if i == 0 { if side { root.rect.x0 - MIND_GAP_X } else { root.rect.x1 + MIND_GAP_X } } else { column.get(i).copied().unwrap_or(0.0) };
            let widest = list.iter().map(|c| width(*c)).fold(0.0, f64::max);
            let mut top = centre.get(i).copied().unwrap_or(0.0) - stack(&span, list) / 2.0;
            for &c in list {
                let s = span.get(c).copied().unwrap_or(0.0);
                let mid = top + s / 2.0;
                let at = Point::new(if side { x - width(c) } else { x }, mid - height(c) / 2.0);
                top += s + MIND_GAP_Y;
                if !(at.x.is_finite() && at.y.is_finite() && mid.is_finite()) {
                    continue;
                }
                if let (Some(m), Some(col), Some(o)) = (centre.get_mut(c), column.get_mut(c), ok.get_mut(c)) {
                    *m = mid;
                    *col = if side { x - widest - MIND_GAP_X } else { x + widest + MIND_GAP_X };
                    *o = true;
                }
                if let Some(b) = nodes.get(c) {
                    out.push((b.id, at));
                }
            }
        }
    }
    out
}
