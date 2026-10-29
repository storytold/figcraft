//! Paints used by the selection, including descendants and effective text-range fills.
//! Targets name paint slots rather than colors: a live edit must not capture other slots when
//! its color happens to become equal to another selection color.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_color::{Paint, PaintKind};
use figcraft_doc::{Document, NodeId, NodeKind, StyleKind, VariableType, VariableValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cmd::lists::{BoundList, list_binding, rebind_edited_paint};
use crate::cmd::styles::{rebind_paint_alpha, unbind_slot};
use crate::params::{get, paint_of, parse, req_string, targets};
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
            "selection.bindColorVariable",
            "Apply selection color variable",
            "{targets, variable (id or name)}; binds the color variable to matching solid fill/stroke slots, replacing a color style there (text ranges with fills of their own take its value in the layer's modes)",
            true,
            bind_variable
        ),
        spec!(
            "selection.detachColor",
            "Detach selection color",
            "{targets}; unlinks the color style or variable of matching fill/stroke lists, keeping their values",
            true,
            detach
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
        // Variables detach only where the edit changed their value; range fills override the node's
        // paints, whose bindings still belong to the other characters.
        if let Some((start, end)) = target.range {
            if let NodeKind::Text(t) = &mut n.kind {
                t.restyle(start, end, |style| style.fills = Some(paints.clone()));
            }
            continue;
        }
        if target.fills {
            n.fills = paints;
            n.styles.fill = None;
        } else {
            n.strokes = paints;
            n.styles.stroke = None;
        }
        rebind_edited_paint(n, target.fills, target.index, &old);
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

/// The binding key (`fills/<i>/color` or `strokes/<i>/color`) of the slot a target stands for, when the
/// target is the node's own list or a text range that shows the node's fills. Returns None for a range
/// with fills of its own, since those fills hold no link.
fn list_field(doc: &Document, target: &PaintTarget) -> Option<String> {
    if let Some((start, _)) = target.range
        && doc.node(target.id)?.kind.text()?.style_at(start).fills.is_some()
    {
        return None;
    }
    let list = if target.fills { BoundList::Fills } else { BoundList::Strokes };
    Some(format!("{}/{}/color", list.prefix(), target.index))
}

/// Why `variable` can't be applied to `targets` (`selection.bindColorVariable`), if it can't: a
/// variable that isn't a color, or a gradient. Scopes don't refuse, as with `variable.bind`; they
/// only filter what the menu offers ([`in_scope`]).
pub fn bind_refusal(doc: &Document, targets: &[PaintTarget], variable: &str) -> Option<String> {
    let Some(var) = doc.variables.get(variable) else { return Some("no such variable".into()) };
    if var.kind != VariableType::Color {
        return Some("a color variable is required".into());
    }
    for target in targets {
        if !target.paint(doc).is_some_and(|paint| matches!(paint.kind, PaintKind::Solid { .. })) {
            return Some("color variables bind to solid paints".into());
        }
        let list = if target.fills { BoundList::Fills } else { BoundList::Strokes };
        let field = format!("{}/{}/color", list.prefix(), target.index);
        if !crate::cmd::variables::field_takes(doc, target.id, &field, var.kind) {
            return Some(format!("`{field}` doesn't take a color variable"));
        }
    }
    None
}

/// Whether `variable`'s scopes cover every slot of `targets` (a variable hidden from pickers has
/// none), so the Selection colors menu offers it there.
pub fn in_scope(doc: &Document, targets: &[PaintTarget], variable: &str) -> bool {
    let Some(var) = doc.variables.get(variable) else { return false };
    targets.iter().all(|target| {
        let list = if target.fills { BoundList::Fills } else { BoundList::Strokes };
        let field = format!("{}/{}/color", list.prefix(), target.index);
        crate::cmd::variables::field_scope(doc, target.id, &field).is_none_or(|scope| var.in_scope(scope))
    })
}

fn bind_variable(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "selection.bindColorVariable";
    let targets = checked_targets(s, p, CMD)?;
    let name = req_string(CMD, p, "variable")?;
    let doc = s.doc()?;
    if let Some(why) = bind_refusal(doc, &targets, &name) {
        return Err(bad(CMD, why));
    }
    let vid = doc.variables.get(&name).map(|v| v.id.clone()).ok_or_else(|| bad(CMD, "no such variable"))?;
    for target in &targets {
        if let Some(field) = list_field(s.doc()?, target) {
            let n = s.doc_mut()?.get_mut(target.id)?;
            if target.fills {
                n.styles.fill = None;
            } else {
                n.styles.stroke = None;
            }
            if !unbind_slot(n, target.fills, target.index) {
                rebind_paint_alpha(&mut n.fills, &mut n.strokes, &field, true);
            }
            n.bound.insert(field, vid.clone());
            continue;
        }
        // A range's own fills can't hold a binding: they take the value, as a range edit does.
        let doc = s.doc()?;
        let Some(VariableValue::Color(color)) = doc.variables.resolve(&vid, &doc.modes_for(target.id)) else { continue };
        let mut paints = target.paints(doc).ok_or_else(|| bad(CMD, "invalid target"))?;
        let Some(slot) = paints.get_mut(target.index) else { return Err(bad(CMD, "invalid paint index")) };
        let own = if slot.color_alpha > 0.0 { (slot.opacity / slot.color_alpha).clamp(0.0, 1.0) } else { 1.0 };
        slot.kind = PaintKind::Solid { color: color.with_alpha(1.0) };
        slot.opacity = own * color.a;
        slot.color_alpha = color.a;
        let Some((start, end)) = target.range else { continue };
        if let NodeKind::Text(t) = &mut s.doc_mut()?.get_mut(target.id)?.kind {
            t.restyle(start, end, |style| style.fills = Some(paints.clone()));
        }
    }
    Ok(json!({"ok": true}))
}

fn detach(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = checked_targets(s, p, "selection.detachColor")?;
    for target in &targets {
        let Some(field) = list_field(s.doc()?, target) else { continue };
        let n = s.doc_mut()?.get_mut(target.id)?;
        if target.fills {
            n.styles.fill = None;
        } else {
            n.styles.stroke = None;
        }
        if unbind_slot(n, target.fills, target.index) {
            rebind_paint_alpha(&mut n.fills, &mut n.strokes, &field, false);
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
