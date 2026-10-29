//! Components and instances.
//!
//! An instance's layers are real nodes copied from its main component; each copy records the
//! main-component node it mirrors (`source`) and which property groups were changed locally
//! (`overrides`). When a main component changes, every instance is rebuilt from it, keeping layer
//! ids and re-applying overrides (matched by source, then by layer name path, so switching
//! variants keeps compatible overrides). Variants are components inside a component set, named
//! `Property=Value, …`; component properties (boolean, text, instance swap) are applied through
//! `prop_refs` on the layers they control.
//!
//! Override groups: `fills`, `strokes`, `effects`, `opacity`, `visible`, `name`, `corners`,
//! `characters`, `textStyle` (font, size, spacing, resizing, …), `padding`, `gap` and `alignment`
//! (of auto layout: its direction and other settings are the main's), `grids`, `exports`,
//! `interactions`, `size` and `resizing` (hug, fill or fixed, with min and max sizes) of layers
//! inside an instance, `modes` (the variable modes a layer inside an instance chooses),
//! `component` (a swapped nested instance), `props` (property values of a nested instance) and
//! `slot` (a slot holding the instance's own content). An instance's own export settings,
//! interactions, resizing and variable modes are its own, like its place. `position`
//! overrides of layers inside an instance are still read and kept, but new ones are refused
//! ([`check_override_edit`]): those layers take their place and constraints from the main.
//!
//! Switching variants keeps an override where the layer's name path matches and the two variants
//! agreed on that property (text layers keep all of theirs); swapping from the Assets panel keeps
//! only text layers' overrides ([`keep_text_overrides`]).
//!
//! Instances never gain or lose layers of their own: commands that would add, remove or reorder
//! layers inside an instance are refused ([`check_structure_edit`]), except inside **slots**. A
//! slot is a frame of a main component bound to a slot property (`prop_refs["slot"]`). An
//! instance's copy of it shows the main's default content until its layers are edited; from then
//! on (override group `slot`) its children are the instance's own content: free to edit, kept
//! across syncs and variant switches, and outside the instance's structure (instances placed in it
//! stand alone).

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::{
    ChildLayout, ContentAlign, CrossAlign, DocError, Document, MainAlign, Node, NodeId, NodeKind, PropDef, Sizing, TextAutoResize, TextData,
};
use figcraft_geom::Affine;
use serde_json::Value;

mod cycles;
mod diff;
mod guard;
mod instances;
mod patch;
mod rebase;
mod sync;
mod variants;

pub use cycles::{
    break_component_cycles, check_component_cycles, component_cycle, contains_itself_error, cycle_text, new_cycle, would_contain_itself,
};
pub use diff::{diff_groups, property_owners, set_boolean_default, variable_groups};
pub use guard::{check_override_edit, check_structure_edit, refused_override, unlink_layers};
pub(crate) use instances::copy_group;
pub use instances::{PROP_BINDING, copy_in_file, create_instance, create_instance_at, sync_instance};
pub use patch::sync_after_edit;
#[cfg(test)]
pub(crate) use patch::{FULL_REBUILD, PATCHED};
#[cfg(test)]
use rebase::RebaseSources;
pub use rebase::rebase_instance;
pub use sync::keep_text_overrides;
pub use variants::{default_variant, exposed_copies, find_variant, mirror_issues, variant_conflicts, variant_name, variant_options, variant_props};

/// Nesting limit for instances inside instances.
const MAX_DEPTH: usize = 16;
/// The override group (and `prop_refs` field) of slots.
pub const SLOT: &str = "slot";
/// Every override group a layer can carry.
pub const OVERRIDE_GROUPS: &[&str] = &[
    "fills",
    "strokes",
    "effects",
    "opacity",
    "visible",
    "name",
    "corners",
    "characters",
    "textStyle",
    "padding",
    "gap",
    "alignment",
    "grids",
    "exports",
    "interactions",
    "size",
    "resizing",
    "position",
    "modes",
    "component",
    "props",
    SLOT,
];

fn is_instance(doc: &Document, id: NodeId) -> bool {
    doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Instance { .. }))
}

/// A slot (inside an instance) whose children are the instance's own content.
fn holds_content(n: &Node) -> bool {
    n.overrides.contains(SLOT)
}

/// `id` and its ancestors, up to the nearest slot above it that holds an instance's own content:
/// content belongs to no instance (it edits freely, and instances placed in it stand alone).
fn scope(doc: &Document, id: NodeId) -> Vec<NodeId> {
    let mut out = vec![id];
    for a in doc.ancestors(id) {
        if doc.node(a).is_some_and(holds_content) {
            break;
        }
        out.push(a);
    }
    out
}

/// The component a node belongs to (itself, or the nearest component ancestor).
pub fn owning_component(doc: &Document, id: NodeId) -> Option<NodeId> {
    let mut chain = vec![id];
    chain.extend(doc.ancestors(id));
    chain.into_iter().find(|c| doc.node(*c).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. })))
}

/// The innermost instance containing `id` (`id` itself when it is an instance).
pub fn owning_instance(doc: &Document, id: NodeId) -> Option<NodeId> {
    scope(doc, id).into_iter().find(|c| is_instance(doc, *c))
}

/// The outermost instance containing `id` (`id` itself when it is a top-level instance).
pub fn root_instance(doc: &Document, id: NodeId) -> Option<NodeId> {
    scope(doc, id).into_iter().rfind(|c| is_instance(doc, *c))
}

/// The innermost instance `id` is a layer of (never `id` itself).
fn enclosing_instance(doc: &Document, id: NodeId) -> Option<NodeId> {
    scope(doc, id).into_iter().skip(1).find(|c| is_instance(doc, *c))
}

/// Whether `id` is a layer inside an instance (not the outermost instance itself).
pub fn inside_instance(doc: &Document, id: NodeId) -> bool {
    enclosing_instance(doc, id).is_some()
}

/// Mark the `modes` override on layers inside instances whose variable modes differ from their
/// main layer's, for files from before `modes` was an override group. Copies follow their main
/// layer's modes, so a layer on other modes chose them itself.
pub fn mark_layer_modes(doc: &mut Document) {
    let chose: Vec<NodeId> = doc
        .nodes()
        .filter(|n| !n.overrides.contains("modes") && n.source.and_then(|s| doc.node(s)).is_some_and(|m| m.var_modes != n.var_modes))
        .map(|n| n.id)
        .filter(|id| inside_instance(doc, *id))
        .collect();
    for id in chose {
        if let Some(n) = doc.node_mut(id) {
            n.overrides.insert("modes".into());
        }
    }
}

/// The slot property frame `id` stands for (its name and preferred components): a frame bound to
/// a slot property of its component, or an instance's copy of one (followed through its sources).
pub fn slot_def(doc: &Document, id: NodeId) -> Option<(&str, &[NodeId])> {
    let mut cur = id;
    for _ in 0..=MAX_DEPTH {
        let n = doc.node(cur)?;
        if !matches!(n.kind, NodeKind::Frame) {
            return None;
        }
        if let Some(name) = n.prop_refs.get(SLOT) {
            let NodeKind::Component { props, .. } = &doc.node(owning_component(doc, cur)?)?.kind else { return None };
            return match props.get(name) {
                Some(PropDef::Slot { preferred }) => Some((name.as_str(), preferred.as_slice())),
                _ => None,
            };
        }
        cur = n.source?;
    }
    None
}

/// The name of the slot property frame `id` stands for ([`slot_def`]).
pub fn slot_name(doc: &Document, id: NodeId) -> Option<String> {
    slot_def(doc, id).map(|(name, _)| name.to_string())
}

/// Whether `id` is a slot (in a main component, or an instance's copy of one).
pub fn is_slot(doc: &Document, id: NodeId) -> bool {
    slot_def(doc, id).is_some()
}

/// Whether slot `id` (inside an instance) holds the instance's own content rather than its main's
/// default content.
pub fn slot_edited(doc: &Document, id: NodeId) -> bool {
    doc.node(id).is_some_and(holds_content)
}

/// The slot `id` is, or lies in, within its instance: where layers may be added, removed and
/// reordered freely.
pub fn slot_scope(doc: &Document, id: NodeId) -> Option<NodeId> {
    let mut chain = vec![id];
    chain.extend(doc.ancestors(id));
    for c in chain {
        if is_instance(doc, c) {
            return None;
        }
        if is_slot(doc, c) {
            return Some(c);
        }
    }
    None
}

/// Whether layers can't be added to, removed from or reordered in `id`: it is (inside) an
/// instance, outside its slots.
pub fn structure_locked(doc: &Document, id: NodeId) -> bool {
    owning_instance(doc, id).is_some() && slot_scope(doc, id).is_none()
}

/// For tree walks that look for where layers can go: whether the children of `id` are locked (in
/// an instance, outside its slots), given whether `id` itself is (`locked`).
pub fn locks_children(doc: &Document, id: NodeId, locked: bool) -> bool {
    is_instance(doc, id) || (locked && !is_slot(doc, id))
}

/// Whether layer `id`'s place (parent and position among its siblings) changed between two
/// versions only where instance structure allows it.
pub fn move_allowed(before: &Document, after: &Document, id: NodeId) -> bool {
    let place = |d: &Document| d.node(id).and_then(|n| n.parent).map(|p| (p, d.index_in_parent(id)));
    let (was, is) = (place(before), place(after));
    was == is || (!was.is_some_and(|(p, _)| structure_locked(before, p)) && !is.is_some_and(|(p, _)| structure_locked(after, p)))
}

/// The slots of a component or instance (its own: not those of instances nested in it, nor of the
/// content put into its slots), with their property names, top-most first.
pub fn slots_of(doc: &Document, root: NodeId) -> Vec<(NodeId, String)> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = doc.node(root).map(|n| n.children.iter().rev().copied().collect()).unwrap_or_default();
    let mut seen = 0usize;
    while let Some(c) = stack.pop() {
        seen += 1;
        if seen > 1_000_000 {
            break;
        }
        let Some(n) = doc.node(c) else { continue };
        if let Some(name) = slot_name(doc, c) {
            out.push((c, name));
        }
        if !is_instance(doc, c) && !holds_content(n) {
            stack.extend(n.children.iter().rev().copied());
        }
    }
    out
}

/// All instances of `component` in the document.
pub fn instances_of(doc: &Document, component: NodeId) -> Vec<NodeId> {
    doc.nodes().filter(|n| matches!(n.kind, NodeKind::Instance { component: c, .. } if c == component)).map(|n| n.id).collect()
}

/// The component set a variant belongs to.
pub fn variant_set(doc: &Document, component: NodeId) -> Option<NodeId> {
    doc.node(component).and_then(|c| c.parent).filter(|p| doc.node(*p).is_some_and(|n| matches!(n.kind, NodeKind::ComponentSet { .. })))
}

/// The layer name a fresh instance of `component` gets: the set's name for variants.
pub fn default_instance_name(doc: &Document, component: NodeId) -> String {
    let set = variant_set(doc, component).and_then(|s| doc.node(s));
    set.or_else(|| doc.node(component)).map(|n| n.name.clone()).unwrap_or_else(|| "Instance".into())
}

/// Default property values of a component (slots hold content, not values: they have none).
pub fn default_props(comp: &Node) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    if let NodeKind::Component { props, .. } = &comp.kind {
        for (k, d) in props {
            if takes_value(d) {
                out.insert(k.clone(), prop_default_value(d));
            }
        }
    }
    out
}

/// Component `c`'s property definitions, in the order they show ([`figcraft_doc::ordered_props`]).
pub fn prop_list(doc: &Document, c: NodeId) -> Vec<(String, PropDef)> {
    match doc.node(c).map(|n| &n.kind) {
        Some(NodeKind::Component { props, prop_order, .. }) => {
            figcraft_doc::ordered_props(props, prop_order).into_iter().map(|(k, d)| (k.clone(), d.clone())).collect()
        }
        _ => Vec::new(),
    }
}

/// Whether instances store a value for the property (every kind but slots).
pub fn takes_value(d: &PropDef) -> bool {
    !matches!(d, PropDef::Slot { .. })
}

/// A property definition's default as a JSON value.
pub fn prop_default_value(d: &PropDef) -> Value {
    match d {
        PropDef::Boolean { default } => Value::Bool(*default),
        PropDef::Text { default } => Value::String(default.clone()),
        PropDef::InstanceSwap { default, .. } => Value::from(default.0),
        PropDef::Variant { default, .. } => Value::String(default.clone()),
        PropDef::Slot { .. } => Value::Null,
    }
}

#[cfg(test)]
mod rebase_source_tests;

#[cfg(test)]
pub(crate) mod tests;
