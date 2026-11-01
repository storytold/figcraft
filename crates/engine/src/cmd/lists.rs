//! A layer's ordered lists (fills, strokes, effects, layout grids): reordering them, and keeping
//! the variable bindings on their items (`fills/<i>/color`, `fills/<i>/stops/<j>/color`,
//! `effects/<i>/radius`, `layoutGrids/<i>/gutter`, …) with their items when a list is reordered
//! or shrinks, and on gradient stops when a stop is removed.
//!
//! Every list is stored bottom first: index 0 paints (or applies) first, the last one is on top;
//! the Design panel shows them in reverse, the topmost row first.

use figcraft_doc::{Document, LayoutGrid, Node, NodeId};
use serde_json::Value;

use super::{CommandSpec, ok};
use crate::params::{req_index, req_targets};
use crate::{Result, Session, spec};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("fill.move", "Move fill", "{ids?, from, to} (indices in the fill list, bottom first)", true, |s, p| {
            move_item(s, p, "fill.move", BoundList::Fills)
        }),
        spec!("stroke.move", "Move stroke", "{ids?, from, to} (indices in the stroke list, bottom first)", true, |s, p| {
            move_item(s, p, "stroke.move", BoundList::Strokes)
        }),
        spec!("effect.move", "Move effect", "{ids?, from, to} (indices in the effect list, bottom first)", true, |s, p| {
            move_item(s, p, "effect.move", BoundList::Effects)
        }),
        spec!("grid.move", "Move layout grid", "{ids?, from, to} (indices in the layout grid list, bottom first)", true, |s, p| {
            move_item(s, p, "grid.move", BoundList::Grids)
        }),
    ]
}

/// One of a layer's ordered lists whose items carry variable bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundList {
    Fills,
    Strokes,
    Effects,
    Grids,
}

impl BoundList {
    /// The binding keys' prefix (`<prefix>/<index>/<field>`).
    pub fn prefix(self) -> &'static str {
        match self {
            BoundList::Fills => "fills",
            BoundList::Strokes => "strokes",
            BoundList::Effects => "effects",
            BoundList::Grids => "layoutGrids",
        }
    }

    fn len(self, n: &Node) -> usize {
        match self {
            BoundList::Fills => n.fills.len(),
            BoundList::Strokes => n.strokes.len(),
            BoundList::Effects => n.effects.len(),
            BoundList::Grids => n.grids.len(),
        }
    }

    /// Unlink the style that defined the whole list (it no longer matches it).
    fn detach_style(self, n: &mut Node) {
        match self {
            BoundList::Fills => n.styles.fill = None,
            BoundList::Strokes => n.styles.stroke = None,
            BoundList::Effects => n.styles.effect = None,
            BoundList::Grids => n.styles.grid = None,
        }
    }

    /// The binding key of `field` on item `index` (`fills/2/color`).
    pub fn key(self, index: usize, field: &str) -> String {
        format!("{}/{index}/{field}", self.prefix())
    }
}

/// The list item a binding key refers to: its list, index and field. `fill`, `fills` and
/// `fills/0` (and `stroke`) are the first paint's colour.
pub fn list_binding(key: &str) -> Option<(BoundList, usize, &str)> {
    match key {
        "fill" | "fills" | "fills/0" => return Some((BoundList::Fills, 0, "color")),
        "stroke" => return Some((BoundList::Strokes, 0, "color")),
        _ => {}
    }
    let (prefix, rest) = key.split_once('/')?;
    let list = [BoundList::Fills, BoundList::Strokes, BoundList::Effects, BoundList::Grids].into_iter().find(|l| l.prefix() == prefix)?;
    let (i, field) = rest.split_once('/')?;
    // Plain digits only (no sign, no "+1").
    if i.is_empty() || !i.bytes().all(|b| b.is_ascii_digit()) || field.is_empty() {
        return None;
    }
    Some((list, i.parse().ok()?, field))
}

/// The binding key on paint `index`'s colour that `n` uses, if any (`fill` for the first fill's,
/// or `fills/<index>/color`).
pub fn paint_binding_key(n: &Node, fills: bool, index: usize) -> Option<&str> {
    let list = if fills { BoundList::Fills } else { BoundList::Strokes };
    n.bound.keys().map(String::as_str).find(|k| list_binding(k) == Some((list, index, "color")))
}

/// The gradient stop a paint field names (`stops/<j>/color` → j).
pub fn stop_binding(field: &str) -> Option<usize> {
    let j = field.strip_prefix("stops/")?.strip_suffix("/color")?;
    if j.is_empty() || !j.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    j.parse().ok()
}

/// Keep the gradient stop bindings of paint `index` of `n`'s fills (or strokes) on their stops after
/// the paint changed from `old`: one stop removed shifts the later ones down (the removed one's
/// binding goes); a paint that is no longer a gradient drops them. Other edits keep stop indices.
/// Returns the removed stop's index.
pub(crate) fn remap_stop_bindings(n: &mut Node, fills: bool, index: usize, old: &figcraft_color::Paint) -> Option<usize> {
    use figcraft_color::PaintKind;
    let list = if fills { BoundList::Fills } else { BoundList::Strokes };
    let new = if fills { n.fills.get(index) } else { n.strokes.get(index) };
    // Some(k): stop k was removed; None: the paint has no stops any more.
    let removed = match (&old.kind, new.map(|p| &p.kind)) {
        (PaintKind::Gradient(a), Some(PaintKind::Gradient(b))) if a.stops.len() == b.stops.len() + 1 => {
            // The first stop that differs is the removed one when the rest moved down unchanged.
            let k = a.stops.iter().zip(&b.stops).position(|(x, y)| x != y).unwrap_or(b.stops.len());
            if a.stops.get(k + 1..) != b.stops.get(k..) {
                return None;
            }
            Some(k)
        }
        (_, Some(PaintKind::Gradient(_))) => return None,
        _ => None,
    };
    let keys: Vec<(String, Option<String>)> = n
        .bound
        .keys()
        .filter_map(|key| {
            let (l, i, field) = list_binding(key).filter(|(l, i, _)| *l == list && *i == index)?;
            let j = stop_binding(field)?;
            match removed.and_then(|k| removed_index(j, k)) {
                Some(to) if to == j => None,
                to => Some((key.clone(), to.map(|to| l.key(i, &format!("stops/{to}/color"))))),
            }
        })
        .collect();
    let mut kept = Vec::new();
    for (old_key, new_key) in keys {
        if let Some(var) = n.bound.remove(&old_key)
            && let Some(k) = new_key
        {
            kept.push((k, var));
        }
    }
    n.bound.extend(kept);
    removed
}

/// After an explicit edit changed paint `index` of `n`'s fills (or strokes) from `old`: its stop
/// bindings follow a removed stop, then each binding whose value the edit changed (the colour, the
/// opacity, a stop's colour) lets go, so the edit stays instead of being put back from the
/// variable. Bindings whose value the edit kept stay; a paint that is gone takes its bindings.
pub(crate) fn rebind_edited_paint(n: &mut Node, fills: bool, index: usize, old: &figcraft_color::Paint) {
    let removed = remap_stop_bindings(n, fills, index, old);
    let mut old = old.clone();
    if let (Some(k), figcraft_color::PaintKind::Gradient(g)) = (removed, &mut old.kind)
        && k < g.stops.len()
    {
        g.stops.remove(k);
    }
    let list = if fills { BoundList::Fills } else { BoundList::Strokes };
    let new = if fills { n.fills.get(index) } else { n.strokes.get(index) }.cloned();
    n.bound.retain(|key, _| match list_binding(key) {
        Some((l, i, field)) if l == list && i == index => new.as_ref().is_some_and(|new| paint_field_kept(&old, new, field)),
        _ => true,
    });
}

/// Whether a paint's bound `field` (`color`, `opacity`, `stops/<j>/color`, …) has the same value in
/// `new` as in `old`.
fn paint_field_kept(old: &figcraft_color::Paint, new: &figcraft_color::Paint, field: &str) -> bool {
    use figcraft_color::PaintKind;
    match field {
        "color" => matches!((&old.kind, &new.kind), (PaintKind::Solid { color: a }, PaintKind::Solid { color: b }) if a == b),
        "opacity" => old.opacity == new.opacity,
        _ => match (stop_binding(field), &old.kind, &new.kind) {
            (Some(j), PaintKind::Gradient(a), PaintKind::Gradient(b)) => a.stops.get(j).map(|s| s.color) == b.stops.get(j).map(|s| s.color),
            (Some(_), _, _) => false,
            _ => old == new,
        },
    }
}

/// Where item `k` lands when the item at `from` moves to `to`.
pub fn moved_index(k: usize, from: usize, to: usize) -> usize {
    if k == from {
        return to;
    }
    let k = if k > from { k - 1 } else { k };
    if k >= to { k + 1 } else { k }
}

/// Where item `k` lands when the item at `removed` goes (None: it is the one removed).
pub fn removed_index(k: usize, removed: usize) -> Option<usize> {
    match k.cmp(&removed) {
        std::cmp::Ordering::Less => Some(k),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(k - 1),
    }
}

/// Keep `n`'s bindings on `list`'s items with their items when the list changes: `map` gives an
/// item's new index, or None when it was removed (its bindings go with it). Bindings whose item
/// keeps its place keep their key.
pub(crate) fn remap_list_bindings(n: &mut Node, list: BoundList, map: impl Fn(usize) -> Option<usize>) {
    let moved: Vec<(String, Option<String>)> = n
        .bound
        .keys()
        .filter_map(|k| {
            let (l, i, field) = list_binding(k).filter(|(l, _, _)| *l == list)?;
            match map(i) {
                Some(j) if j == i => None,
                Some(j) => Some((k.clone(), Some(l.key(j, field)))),
                None => Some((k.clone(), None)),
            }
        })
        .collect();
    let mut kept = Vec::new();
    for (old, new) in moved {
        if let Some(var) = n.bound.remove(&old)
            && let Some(new) = new
        {
            kept.push((new, var));
        }
    }
    n.bound.extend(kept);
}

/// Move `v[from]` to `to`; false (and nothing changes) when either is out of range or they match.
fn reorder<T>(v: &mut Vec<T>, from: usize, to: usize) -> bool {
    if from >= v.len() || to >= v.len() || from == to {
        return false;
    }
    let item = v.remove(from);
    v.insert(to, item);
    true
}

/// `fill.move` / `stroke.move` / `effect.move` / `grid.move`: the item keeps everything (an image
/// fill its image and transform, a gradient its stops) and its bindings go with it.
fn move_item(s: &mut Session, p: &Value, cmd: &str, list: BoundList) -> Result<Value> {
    let from = req_index(cmd, p, "from")?;
    let to = req_index(cmd, p, "to")?;
    for id in req_targets(cmd, s, p)? {
        let len = s.doc()?.node(id).map(|n| list.len(n)).unwrap_or(0);
        if from >= len || to >= len || from == to {
            continue;
        }
        let n = s.doc_mut()?.get_mut(id)?;
        let moved = match list {
            BoundList::Fills => reorder(&mut n.fills, from, to),
            BoundList::Strokes => reorder(&mut n.strokes, from, to),
            BoundList::Effects => reorder(&mut n.effects, from, to),
            BoundList::Grids => reorder(&mut n.grids, from, to),
        };
        if moved {
            remap_list_bindings(n, list, |k| Some(moved_index(k, from, to)));
            list.detach_style(n);
        }
    }
    ok()
}

/// Drop item `index` of `list` on `n` with its bindings (later items' bindings move down one).
/// False when there is no such item.
pub(crate) fn remove_item(n: &mut Node, list: BoundList, index: usize) -> bool {
    if index >= list.len(n) {
        return false;
    }
    match list {
        BoundList::Fills => {
            let _ = n.fills.remove(index);
        }
        BoundList::Strokes => {
            let _ = n.strokes.remove(index);
        }
        BoundList::Effects => {
            let _ = n.effects.remove(index);
        }
        BoundList::Grids => {
            let _ = n.grids.remove(index);
        }
    }
    remap_list_bindings(n, list, |k| removed_index(k, index));
    true
}

/// Set `field` (`size|count|gutter|offset`) of layout grid `i` to a bound number.
pub(crate) fn set_grid_field(grids: &mut [LayoutGrid], i: usize, field: &str, x: f64) {
    let Some(g) = grids.get_mut(i).filter(|_| x.is_finite()) else { return };
    match field {
        "size" => g.size = x.max(0.0),
        "count" => g.count = x.round().clamp(1.0, 1000.0) as u32,
        "gutter" => g.gutter = x.max(0.0),
        "offset" => g.offset = x,
        _ => {}
    }
}

/// Write bound numbers into a node's layout grids (`layoutGrids/<i>/<size|count|gutter|offset>`;
/// only what changed, so unchanged layers stay shared).
pub(crate) fn apply_grid_bindings(doc: &mut Document, id: NodeId, sets: &[(usize, String, f64)]) {
    let Some(mut grids) = doc.node(id).map(|n| n.grids.clone()) else { return };
    for (i, field, x) in sets {
        set_grid_field(&mut grids, *i, field, *x);
    }
    if doc.node(id).is_some_and(|n| n.grids != grids)
        && let Ok(n) = doc.get_mut(id)
    {
        n.grids = grids;
    }
}
