//! Prototype actions run by the prototype player on its own copy of the document: set variable (a
//! value or an expression), set variable mode, choosing the branch of a conditional action, where
//! navigating to a section goes, which instance "Change to" changes, the interactions instances
//! play (interactive components), and the hotspots a screen offers.

use std::collections::HashMap;

use figcraft_doc::proto::{Action, ConditionalBlock, Expr, Interaction};
use figcraft_doc::{Document, NodeId, NodeKind, VariableType, VariableValue};
use figcraft_geom::Rect;
use serde_json::Value;

use crate::cmd::styles::apply_bindings;
use crate::params::{color_of, f64_of};

/// A JSON value as a value of a `kind` variable (`None` when it doesn't fit).
pub fn literal(kind: VariableType, v: &Value) -> Option<VariableValue> {
    Some(match kind {
        VariableType::Color => VariableValue::Color(color_of(v)?),
        VariableType::Number => VariableValue::Number(f64_of(v).filter(|n| n.is_finite())?),
        VariableType::Boolean => VariableValue::Boolean(match v {
            Value::Bool(b) => *b,
            Value::String(s) => s == "true",
            Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
            _ => return None,
        }),
        VariableType::String => VariableValue::String(match v {
            Value::String(s) => s.clone(),
            Value::Bool(_) | Value::Number(_) => v.to_string(),
            _ => return None,
        }),
    })
}

/// An expression result as a value of a `kind` variable: numbers and booleans become text for
/// string variables; other mismatches don't fit.
fn fit(kind: VariableType, v: VariableValue) -> Option<VariableValue> {
    match (kind, v) {
        (VariableType::String, v @ (VariableValue::Number(_) | VariableValue::Boolean(_))) => Some(VariableValue::String(v.display())),
        (k, v) if v.type_of() == Some(k) => Some(v),
        _ => None,
    }
}

/// Set stored variable id `variable` to `value`, or to `expr` evaluated in the modes of `node`
/// (the layer whose interaction runs), in every mode, and re-apply bindings. False when nothing
/// could be set.
pub fn set_variable(doc: &mut Document, node: NodeId, variable: &str, value: &Value, expr: Option<&Expr>) -> bool {
    let Some(var) = doc.variables.get_by_id(variable) else { return false };
    let (id, kind) = (var.id.clone(), var.kind);
    let v = match expr {
        Some(e) => e.eval(&doc.variables, &doc.modes_for(node)).and_then(|v| fit(kind, v)),
        None => literal(kind, value),
    };
    let Some(v) = v else { return false };
    let Some(slot) = doc.variables.variables.iter_mut().find(|x| x.id == id) else { return false };
    for val in slot.values.values_mut() {
        *val = v.clone();
    }
    apply_bindings(doc);
    crate::style_vars::refresh(doc, None, None);
    true
}

/// Switch `collection` to `mode` (ids or names) for the prototype `frame` is in: on its page, and
/// on the page's top-level frames that chose another mode of it (modes chosen deeper inside still
/// win), then re-apply bindings. False for an unknown collection or mode.
pub fn set_mode(doc: &mut Document, frame: NodeId, collection: &str, mode: &str) -> bool {
    let Some(c) = doc.variables.collection(collection) else { return false };
    let Some(m) = c.find_mode(mode) else { return false };
    let (collection, mode) = (c.id.clone(), m.id.clone());
    set_mode_by_id(doc, frame, &collection, &mode)
}

/// Switch a stored collection id to a stored mode id (a prototype action's), without matching
/// another collection's or mode's name (#1613). Otherwise behaves like [`set_mode`].
pub fn set_mode_by_id(doc: &mut Document, frame: NodeId, collection: &str, mode: &str) -> bool {
    let Some(c) = doc.variables.collection_by_id(collection) else { return false };
    let Some(mode) = c.mode_by_id(mode).map(|m| m.id.clone()) else {
        return false;
    };
    let cid = c.id.clone();
    let Some(page) = doc.page_of(frame) else { return false };
    let mut targets: Vec<NodeId> = doc
        .node(page)
        .map(|p| p.children.iter().copied().filter(|t| doc.node(*t).is_some_and(|n| n.var_modes.contains_key(&cid))).collect())
        .unwrap_or_default();
    targets.push(page);
    for id in targets {
        if let Some(n) = doc.node_mut(id) {
            n.var_modes.insert(cid.clone(), mode.clone());
        }
    }
    apply_bindings(doc);
    crate::style_vars::refresh(doc, None, None);
    true
}

/// The actions of the branch of a conditional action that runs, with variables resolved in the
/// modes of `node`.
pub fn branch(doc: &Document, node: NodeId, blocks: &[ConditionalBlock]) -> Vec<Action> {
    ConditionalBlock::choose(blocks, &doc.variables, &doc.modes_for(node)).map(<[Action]>::to_vec).unwrap_or_default()
}

/// The frame "Navigate to" `dest` shows: the top-level frame holding it; for a section, the frame
/// in it shown last (`visited`: section → frame, see [`remember_sections`]), else its first frame
/// in reading order. `None` when there's nothing to show.
pub fn navigate_target(doc: &Document, dest: NodeId, visited: &HashMap<NodeId, NodeId>) -> Option<NodeId> {
    let n = doc.node(dest)?;
    if !matches!(n.kind, NodeKind::Section { .. }) {
        return Some(doc.top_level_frame(dest).unwrap_or(dest));
    }
    visited.get(&dest).copied().filter(|f| doc.contains(*f) && doc.is_ancestor(dest, *f)).or_else(|| doc.frames_in(dest).first().copied())
}

/// Record `frame` as the frame shown last in each section around it.
pub fn remember_sections(doc: &Document, frame: NodeId, visited: &mut HashMap<NodeId, NodeId>) {
    for a in doc.ancestors(frame) {
        if doc.node(a).is_some_and(|n| matches!(n.kind, NodeKind::Section { .. })) {
            visited.insert(a, frame);
        }
    }
}

/// Interactive components: every instance plays the interactions on its main component's root
/// (the variant it shows), except those with a trigger the instance has interactions of its own
/// for (the instance's win). Interactions an instance holds that equal one on a variant of its
/// set are copies (instances used to copy them when placed) and give way to the variant shown.
/// For the player's copy of the document.
pub fn play_interactions(doc: &mut Document) {
    let plays: Vec<(NodeId, Vec<Interaction>)> = doc
        .nodes()
        .filter_map(|n| {
            let NodeKind::Instance { component, .. } = &n.kind else { return None };
            let main = doc.node(*component)?;
            let variants = crate::components::variant_set(doc, *component).and_then(|s| doc.node(s)).map_or(vec![*component], |s| s.children.clone());
            let copied = |it: &Interaction| variants.iter().filter_map(|v| doc.node(*v)).any(|v| v.interactions.contains(it));
            let own: Vec<&Interaction> = n.interactions.iter().filter(|it| !copied(it)).collect();
            let plays: Vec<Interaction> = own
                .iter()
                .map(|it| (*it).clone())
                .chain(main.interactions.iter().filter(|it| own.iter().all(|o| o.trigger != it.trigger)).cloned())
                .collect();
            (plays != n.interactions).then_some((n.id, plays))
        })
        .collect();
    for (id, its) in plays {
        if let Some(n) = doc.node_mut(id) {
            n.interactions = its;
        }
    }
}

/// The instance "Change to" `dest` changes when it runs on `node`: the nearest of `node` and the
/// instances around it that shows a variant of `dest`'s component set (a button's own icon
/// instance doesn't change), else the innermost instance holding `node`.
pub fn change_to_target(doc: &Document, node: NodeId, dest: NodeId) -> Option<NodeId> {
    let set = crate::components::variant_set(doc, dest);
    let in_set = |id: &NodeId| match doc.node(*id).map(|n| &n.kind) {
        Some(NodeKind::Instance { component, .. }) => set.is_some() && crate::components::variant_set(doc, *component) == set,
        _ => false,
    };
    std::iter::once(node).chain(doc.ancestors(node)).find(in_set).or_else(|| crate::components::owning_instance(doc, node))
}

/// The hotspots of the screen `root` (a presented frame or overlay): its visible layers with
/// interactions, bottom-most first, with their bounds on the page clipped by the frames around
/// them that clip content (the presenter flashes them after a click where nothing reacts).
pub fn hotspots(doc: &Document, root: NodeId) -> Vec<(NodeId, Rect)> {
    let mut out = Vec::new();
    // Each visible layer's clip: its bounds cut by the clipping frames around it (inside `root`).
    let mut clips: HashMap<NodeId, Option<Rect>> = HashMap::new();
    for id in doc.subtree(root) {
        let Some(n) = doc.node(id) else { continue };
        let parent_clip = match n.parent.filter(|_| id != root) {
            Some(p) => match clips.get(&p) {
                Some(c) => *c,
                // The parent is hidden.
                None => continue,
            },
            None => None,
        };
        if !n.visible {
            continue;
        }
        let bounds = doc.world_bounds(id);
        let own = match (parent_clip, bounds) {
            (Some(c), Some(b)) if n.clip && n.kind.is_frame_like() => Some(c.intersect(b)),
            (None, Some(b)) if n.clip && n.kind.is_frame_like() => Some(b),
            (c, _) => c,
        };
        clips.insert(id, own);
        if n.interactions.is_empty() {
            continue;
        }
        let Some(b) = bounds else { continue };
        let r = parent_clip.map_or(b, |c| c.intersect(b));
        if r.width() > 0.0 && r.height() > 0.0 {
            out.push((id, r));
        }
    }
    out
}
