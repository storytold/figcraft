//! Paints used by the selection, including descendants and effective text-range fills.
//! Targets name paint slots rather than colors: a live edit must not capture other slots when
//! its color happens to become equal to another selection color.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_color::{Paint, PaintKind};
use figcraft_doc::{Document, NodeId, NodeKind, StyleKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cmd::lists::{BoundList, list_binding};
use crate::params::*;
use crate::{CommandSpec, Result, Session, bad, spec};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaintTarget {
    pub id: NodeId,
    pub fills: bool,
    pub index: usize,
    /// Character offsets, not byte offsets. None means the node's fill or stroke list.
    pub range: Option<(usize, usize)>,
}

impl PaintTarget {
    pub fn paints(&self, doc: &Document) -> Option<Vec<Paint>> {
        let n = doc.node(self.id)?;
        if let Some((start, end)) = self.range {
            let t = n.kind.text()?;
            if !self.fills || start > end || end > t.char_count() || (start == end && end != 0) {
                return None;
            }
            Some(t.style_at(start).fills.clone().unwrap_or_else(|| n.fills.clone()))
        } else {
            Some(if self.fills { n.fills.clone() } else { n.strokes.clone() })
        }
    }

    pub fn paint(&self, doc: &Document) -> Option<Paint> {
        self.paints(doc)?.get(self.index).cloned()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SelectionColor {
    pub paint: Paint,
    pub style: Option<String>,
    pub variable: Option<String>,
    pub targets: Vec<PaintTarget>,
}

/// Deterministic order, one target per slot even when a parent and child are both selected.
/// Images and patterns are assets rather than selection colors. Hidden paints are omitted.
pub fn collect(doc: &Document, ids: &[NodeId]) -> Vec<SelectionColor> {
    let mut groups: Vec<SelectionColor> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut by_paint: BTreeMap<(Option<String>, Option<String>, String), usize> = BTreeMap::new();
    let mut pending: Vec<_> = ids.iter().rev().copied().collect();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(n) = doc.node(id) else { continue };
        if n.is_mask {
            continue;
        }
        if !matches!(n.kind, NodeKind::BooleanOp { .. }) {
            pending.extend(n.children.iter().rev().copied());
        }
        let mut add = |paints: &[Paint], fills: bool, range: Option<(usize, usize)>| {
            for (index, paint) in paints.iter().enumerate() {
                if !paint.visible || !matches!(paint.kind, PaintKind::Solid { .. } | PaintKind::Gradient(_)) {
                    continue;
                }
                let target = PaintTarget { id, fills, index, range };
                let inherited = range.is_none_or(|(start, _)| n.kind.text().is_some_and(|t| t.style_at(start).fills.is_none()));
                let style = inherited.then(|| if fills { n.styles.fill.clone() } else { n.styles.stroke.clone() }).flatten();
                let list = if fills { BoundList::Fills } else { BoundList::Strokes };
                let variable = inherited
                    .then(|| {
                        n.bound.iter().find_map(|(key, var)| {
                            list_binding(key).filter(|(l, i, field)| *l == list && *i == index && *field == "color").map(|_| var.clone())
                        })
                    })
                    .flatten();
                let Ok(value) = serde_json::to_string(paint) else { continue };
                let key = (style.clone(), variable.clone(), value);
                if let Some(index) = by_paint.get(&key).copied() {
                    if let Some(g) = groups.get_mut(index) {
                        g.targets.push(target);
                    }
                } else {
                    by_paint.insert(key, groups.len());
                    groups.push(SelectionColor { paint: paint.clone(), style, variable, targets: vec![target] });
                }
            }
        };
        if let NodeKind::Text(t) = &n.kind {
            let spans = t.spans();
            if spans.iter().all(|(_, _, style)| style.fills.is_none()) {
                add(&n.fills, true, None);
            } else {
                for (start, end, style) in spans {
                    add(style.fills.as_deref().unwrap_or(&n.fills), true, Some((start, end)));
                }
            }
        } else {
            add(&n.fills, true, None);
        }
        add(&n.strokes, false, None);
    }
    groups
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "selection.colors",
            "Selection colors",
            "{ids?} → {colors: [{paint, targets}]}; includes descendants and text ranges",
            false,
            |s, p| { Ok(json!({"colors": collect(s.doc()?, &targets(s, p))})) }
        ),
        spec!("selection.setPaint", "Change selection color", "{targets: [{id, fills, index, range: [start,end] | null}], paint}", true, set_paint),
        spec!(
            "selection.applyColorStyle",
            "Apply selection color style",
            "{targets, style}; applies the color style to matching fill/stroke lists (text ranges keep typography)",
            true,
            apply_style
        ),
        spec!(
            "selection.selectColor",
            "Select layers with color",
            "{targets} → {ids}; selects the layers owning these paint slots",
            false,
            select_color
        ),
    ]
}

fn checked_targets(s: &Session, p: &Value, cmd: &str) -> Result<Vec<PaintTarget>> {
    let ts: Vec<PaintTarget> = parse(cmd, p, "targets")?.ok_or_else(|| bad(cmd, "`targets` is required"))?;
    if ts.is_empty() {
        return Err(bad(cmd, "`targets` must not be empty"));
    }
    for t in &ts {
        if t.paint(s.doc()?).is_none() {
            return Err(bad(cmd, "a selection color target no longer exists"));
        }
    }
    Ok(ts)
}

/// Detach variables only where the explicit edit changed their value. In particular, an
/// opacity edit keeps the color binding and a range edit keeps other characters' node bindings.
fn unbind_changed(n: &mut figcraft_doc::Node, t: &PaintTarget, old: &Paint, new: &Paint) {
    // Range fills override the node's paints; the node binding still belongs to other chars.
    if t.range.is_some() {
        return;
    }
    let list = if t.fills { BoundList::Fills } else { BoundList::Strokes };
    n.bound.retain(|key, _| {
        let Some((l, index, field)) = list_binding(key) else { return true };
        if l != list || index != t.index {
            return true;
        }
        match field {
            "color" => match (&old.kind, &new.kind) {
                (PaintKind::Solid { color: a }, PaintKind::Solid { color: b }) => a == b,
                _ => false,
            },
            "opacity" => old.opacity == new.opacity,
            _ => {
                let stop = field.strip_prefix("stops/").and_then(|s| s.strip_suffix("/color")).and_then(|s| s.parse::<usize>().ok());
                match (stop, &old.kind, &new.kind) {
                    (Some(i), PaintKind::Gradient(a), PaintKind::Gradient(b)) => a.stops.get(i).map(|s| s.color) == b.stops.get(i).map(|s| s.color),
                    (Some(_), _, _) => false,
                    _ => old == new,
                }
            }
        }
    });
}

fn set_paint(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "selection.setPaint";
    let targets = checked_targets(s, p, CMD)?;
    let paint = get(p, "paint").and_then(paint_of).ok_or_else(|| bad(CMD, "`paint` must be a paint"))?;
    for target in &targets {
        let mut paints = target.paints(s.doc()?).ok_or_else(|| bad(CMD, "invalid target"))?;
        let Some(slot) = paints.get_mut(target.index) else { return Err(bad(CMD, "invalid paint index")) };
        if *slot == paint {
            continue;
        }
        let old = slot.clone();
        *slot = paint.clone();
        let n = s.doc_mut()?.get_mut(target.id)?;
        unbind_changed(n, target, &old, &paint);
        if let Some((start, end)) = target.range {
            if let NodeKind::Text(t) = &mut n.kind {
                t.restyle(start, end, |style| style.fills = Some(paints.clone()));
            }
        } else if target.fills {
            n.fills = paints;
            n.styles.fill = None;
        } else {
            n.strokes = paints;
            n.styles.stroke = None;
        }
    }
    Ok(json!({"ok": true}))
}

fn apply_style(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "selection.applyColorStyle";
    let targets = checked_targets(s, p, CMD)?;
    let name = req_string(CMD, p, "style")?;
    let style = s.doc()?.style(&name).cloned().ok_or_else(|| bad(CMD, "no such color style"))?;
    if !matches!(style.kind, StyleKind::Paint { .. }) {
        return Err(bad(CMD, "a color style is required"));
    }
    for target in &targets {
        if let Some((start, end)) = target.range {
            // Text ranges have typography-style links, not paint-style links. Copy resolved paint
            // values just as an explicit range fill does; never replace its font or size.
            let StyleKind::Paint { paints } = crate::style_vars::kind_for(s.doc()?, &style, target.id).into_owned() else { continue };
            let n = s.doc_mut()?.get_mut(target.id)?;
            if let NodeKind::Text(t) = &mut n.kind {
                t.restyle(start, end, |st| st.fills = Some(paints.clone()));
            }
        } else {
            crate::cmd::styles::apply_style_ref(s.doc_mut()?, target.id, if target.fills { "fillStyle" } else { "strokeStyle" }, Some(&style.id))?;
        }
    }
    Ok(json!({"ok": true}))
}

fn select_color(s: &mut Session, p: &Value) -> Result<Value> {
    let ts = checked_targets(s, p, "selection.selectColor")?;
    let ids: Vec<_> = ts.iter().map(|t| t.id).collect::<BTreeSet<_>>().into_iter().collect();
    crate::cmd::edit::select_and_page(s, ids.clone())?;
    Ok(json!({"ids": ids}))
}
