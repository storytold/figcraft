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
//! inside an instance, `component` (a swapped nested instance), `props` (property values of a
//! nested instance) and `slot` (a slot holding the instance's own content). An instance's own
//! export settings, interactions and resizing are its own, like its place. `position`
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

use figcraft_doc::*;
use figcraft_geom::Affine;
use serde_json::Value;

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

/// Create an instance of `component` under `parent` at `(x, y)` (parent space).
pub fn create_instance(doc: &mut Document, component: NodeId, parent: NodeId, x: f64, y: f64) -> Result<NodeId, DocError> {
    create_instance_at(doc, component, parent, x, y, None)
}

/// [`create_instance`] at child `index` of `parent` (None: on top).
pub fn create_instance_at(doc: &mut Document, component: NodeId, parent: NodeId, x: f64, y: f64, index: Option<usize>) -> Result<NodeId, DocError> {
    let comp = doc.get(component)?.clone();
    if !matches!(comp.kind, NodeKind::Component { .. }) {
        return Err(DocError::Invalid(format!("{component} is not a component")));
    }
    if doc.is_ancestor(component, parent) {
        return Err(DocError::Invalid("an instance can't be placed inside its own main component".into()));
    }
    let id = doc.alloc_id();
    let mut root = comp.clone();
    root.id = id;
    root.name = default_instance_name(doc, component);
    root.kind = NodeKind::Instance { component, props: default_props(&comp) };
    root.children.clear();
    root.parent = None;
    root.source = Some(component);
    root.overrides.clear();
    root.transform = Affine::translate((x, y));
    root.prop_refs.clear();
    root.annotations.clear();
    root.code_links.clear();
    // The main's dev resources show on its instances without being copied.
    root.dev_resources.clear();
    // The main's interactions play on its instances in prototypes; the instance's own start empty.
    root.interactions.clear();
    doc.insert(root, parent, index)?;
    sync_instance(doc, id);
    Ok(id)
}

/// Which part of the carried overrides to apply.
#[derive(Clone, Copy, PartialEq)]
enum Carry {
    /// Paint, text, visibility, … (everything but geometry).
    Values,
    /// Values plus size (instance roots: their size decides how children follow constraints).
    Root,
    /// Size and position (children, after constraints ran).
    Geometry,
}

/// Copy the overridden property groups of `from` onto `to`.
fn carry_overrides(from: &Node, to: &mut Node, what: Carry) {
    for g in &from.overrides {
        let geo = matches!(g.as_str(), "size" | "position");
        match what {
            Carry::Values if geo => continue,
            Carry::Root if g == "position" => continue,
            Carry::Geometry if !geo => continue,
            _ => {}
        }
        copy_group(from, to, g);
    }
    if what != Carry::Geometry {
        to.overrides = from.overrides.clone();
    }
}

/// Copy the values of property group `g` of `from` onto `to`, with the variables bound to them.
pub(crate) fn copy_group(from: &Node, to: &mut Node, g: &str) {
    if from.bound.iter().chain(to.bound.iter()).any(|(k, _)| binding_group(k) == Some(g)) {
        to.bound.retain(|k, _| binding_group(k) != Some(g));
        to.bound.extend(bindings_of(from, g).map(|(k, v)| (k.clone(), v.clone())));
    }
    match g {
        "fills" => {
            to.fills = from.fills.clone();
            to.styles.fill = from.styles.fill.clone();
        }
        "strokes" => {
            to.strokes = from.strokes.clone();
            to.stroke = from.stroke.clone();
            to.styles.stroke = from.styles.stroke.clone();
        }
        "effects" => {
            to.effects = from.effects.clone();
            to.styles.effect = from.styles.effect.clone();
        }
        "opacity" => {
            to.opacity = from.opacity;
            to.blend = from.blend;
        }
        "visible" => to.visible = from.visible,
        "name" => to.name = from.name.clone(),
        "corners" => to.corners = from.corners,
        "characters" => {
            if let (NodeKind::Text(a), NodeKind::Text(b)) = (&from.kind, &mut to.kind) {
                b.characters = a.characters.clone();
                b.runs = a.runs.clone();
                b.paragraphs = a.paragraphs.clone();
            }
        }
        "textStyle" => {
            if let (NodeKind::Text(a), NodeKind::Text(b)) = (&from.kind, &mut to.kind) {
                b.style = a.style.clone();
                b.runs = a.runs.clone();
                b.auto_resize = a.auto_resize;
                b.align_h = a.align_h;
                b.align_v = a.align_v;
                b.paragraph_spacing = a.paragraph_spacing;
                b.paragraph_indent = a.paragraph_indent;
                b.list = a.list;
                b.paragraphs = a.paragraphs.clone();
                b.list_spacing = a.list_spacing;
                b.hanging_punctuation = a.hanging_punctuation;
                b.hanging_list = a.hanging_list;
                b.max_lines = a.max_lines;
                b.features = a.features.clone();
            }
            to.styles.text = from.styles.text.clone();
        }
        "padding" | "gap" | "alignment" => {
            if let (Some(a), Some(b)) = (&from.layout, &mut to.layout) {
                match g {
                    "padding" => b.padding = a.padding,
                    "gap" => (b.gap, b.cross_gap) = (a.gap, a.cross_gap),
                    _ => (b.main_align, b.cross_align, b.content_align) = (a.main_align, a.cross_align, a.content_align),
                }
            }
        }
        "grids" => {
            to.grids = from.grids.clone();
            to.styles.grid = from.styles.grid.clone();
        }
        "exports" => to.exports = from.exports.clone(),
        "interactions" => to.interactions = from.interactions.clone(),
        "size" => {
            to.width = from.width;
            to.height = from.height;
            if let (NodeKind::Text(a), NodeKind::Text(b)) = (&from.kind, &mut to.kind) {
                b.auto_resize = a.auto_resize;
            }
        }
        "resizing" => {
            let (a, b) = (&from.child_layout, &mut to.child_layout);
            (b.sizing_h, b.sizing_v, b.min_w, b.max_w, b.min_h, b.max_h) = (a.sizing_h, a.sizing_v, a.min_w, a.max_w, a.min_h, a.max_h);
        }
        "position" => to.transform = from.transform,
        "component" => {
            if let (NodeKind::Instance { component: a, .. }, NodeKind::Instance { component: b, .. }) = (&from.kind, &mut to.kind) {
                *b = *a;
            }
        }
        "props" => {
            if let (NodeKind::Instance { props: a, .. }, NodeKind::Instance { props: b, .. }) = (&from.kind, &mut to.kind) {
                *b = a.clone();
            }
            carry_prop_bindings(from, to);
        }
        _ => {}
    }
}

/// Layer name paths below `root` ("Frame/Label", with `#n` for same-named siblings). With
/// `src_names` (an instance's layers), layers are named after the main layer they mirror (renames
/// don't break matching), and the content of slots holding the instance's own is left out.
fn path_map(doc: &Document, root: NodeId, src_names: bool) -> BTreeMap<NodeId, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![(root, String::new())];
    while let Some((id, prefix)) = stack.pop() {
        let Some(n) = doc.node(id) else { continue };
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for c in &n.children {
            let Some(cn) = doc.node(*c) else { continue };
            // A nested instance linked only to its own component so far (an imported one) mirrors no
            // layer yet: it goes by its own name, like the main's layer it will be matched with.
            let unplaced = matches!(&cn.kind, NodeKind::Instance { component, .. } if cn.source == Some(*component));
            let mirrored = cn.source.filter(|_| src_names && !unplaced).and_then(|s| doc.node(s));
            let name = mirrored.map_or(cn.name.as_str(), |s| s.name.as_str());
            let k = seen.entry(name).or_insert(0);
            let seg = if *k == 0 { name.to_string() } else { format!("{name}#{k}") };
            *k += 1;
            let path = if prefix.is_empty() { seg } else { format!("{prefix}/{seg}") };
            out.insert(*c, path.clone());
            if out.len() > 1_000_000 {
                return out;
            }
            if !(src_names && holds_content(cn)) {
                stack.push((*c, path));
            }
        }
    }
    out
}

/// The binding key prefix of an instance's component property value
/// (`componentProperties/<name>`): a boolean property takes a boolean variable, a text property a
/// string variable.
pub const PROP_BINDING: &str = "componentProperties/";

/// Give `to` the component property bindings of instance `from` (they go with its property
/// values, which an instance keeps across rebuilds).
fn carry_prop_bindings(from: &Node, to: &mut Node) {
    to.bound.retain(|k, _| !k.starts_with(PROP_BINDING));
    to.bound.extend(from.bound.iter().filter(|(k, _)| k.starts_with(PROP_BINDING)).map(|(k, v)| (k.clone(), v.clone())));
}

/// The override group a property-bound field belongs to.
fn field_group(field: &str) -> &'static str {
    match field {
        "visible" => "visible",
        "characters" => "characters",
        "mainComponent" => "component",
        _ => "",
    }
}

/// Apply one component-property value to a layer.
fn apply_prop(doc: &Document, n: &mut Node, field: &str, val: &Value) {
    match field {
        "visible" => {
            if let Some(b) = val.as_bool() {
                n.visible = b;
            }
        }
        "characters" => {
            if let (NodeKind::Text(t), Some(sv)) = (&mut n.kind, val.as_str().map(crate::cmd::props::normalize_line_endings))
                && t.characters != sv
            {
                let len = t.char_count();
                t.replace_range(0, len, &sv);
            }
        }
        "mainComponent" => {
            if let (NodeKind::Instance { component: c, .. }, Some(v)) = (&mut n.kind, crate::params::id_of(val))
                && doc.node(v).is_some_and(|m| matches!(m.kind, NodeKind::Component { .. }))
            {
                *c = v;
            }
        }
        _ => {}
    }
}

/// Rebuild instance `inst` from its main component, keeping ids, overrides and the content of its
/// slots. An instance whose main component was deleted keeps its layers as they are.
pub fn sync_instance(doc: &mut Document, inst: NodeId) {
    // Layers the rebuild leaves as they were keep their shared snapshot (and aren't edits).
    let old: Vec<(std::sync::Arc<Node>, bool)> =
        doc.subtree(inst).into_iter().filter_map(|i| doc.node_arc(i).map(|a| (a.clone(), doc.dirty().contains(&i)))).collect();
    let kept = slot_content(doc, inst);
    sync_instance_depth(doc, inst, 0, BTreeMap::new());
    restore_slot_content(doc, kept);
    for (a, was_dirty) in old {
        doc.keep_if_unchanged(a, was_dirty);
    }
}

/// The slots in instance `inst` (nested instances included) holding the instance's own content:
/// each with its content (the subtrees of its children, pre-order).
fn slot_content(doc: &Document, inst: NodeId) -> Vec<(NodeId, Vec<Node>)> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = doc.node(inst).map(|n| n.children.clone()).unwrap_or_default();
    while let Some(c) = stack.pop() {
        let Some(n) = doc.node(c) else { continue };
        if holds_content(n) {
            let nodes = n.children.iter().flat_map(|k| doc.subtree(*k)).filter_map(|d| doc.node(d).cloned()).collect();
            out.push((c, nodes));
        } else {
            stack.extend(n.children.iter().copied());
        }
    }
    out
}

/// Put slot content back after a rebuild, into the slots that still hold the instance's own
/// content (a slot the main no longer has, or no longer marks as a slot, loses it). Constraints
/// and geometry overrides ran before, so the content keeps its place.
fn restore_slot_content(doc: &mut Document, kept: Vec<(NodeId, Vec<Node>)>) {
    for (slot, nodes) in kept {
        let keep = slot_edited(doc, slot) && is_slot(doc, slot) && nodes.iter().all(|n| !doc.contains(n.id));
        if !keep {
            continue;
        }
        let ids: Vec<NodeId> = nodes.iter().map(|n| n.id).collect();
        for n in nodes {
            // Pre-order: every parent is back before its children.
            let _ = insert_linked(doc, n);
        }
        // Never leave links to a layer that couldn't come back.
        for id in ids.into_iter().chain([slot]) {
            let dangling = doc.node(id).is_some_and(|n| n.children.iter().any(|c| !doc.contains(*c)));
            if dangling {
                let live: Vec<NodeId> = doc.node(id).map(|n| n.children.iter().copied().filter(|c| doc.contains(*c)).collect()).unwrap_or_default();
                if let Some(n) = doc.node_mut(id) {
                    n.children = live;
                }
            }
        }
    }
}

/// `spare`: layers the outer rebuild didn't reuse that were below `inst` (a swapped nested
/// instance): its layers the outer main doesn't have, matched here so they keep ids and overrides.
fn sync_instance_depth(doc: &mut Document, inst: NodeId, depth: usize, spare: BTreeMap<NodeId, std::sync::Arc<Node>>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Some(mut root) = doc.node(inst).cloned() else { return };
    let NodeKind::Instance { component, props: root_props } = &root.kind else { return };
    let component = *component;
    let Some(comp) = doc.node(component).cloned() else { return };
    if !matches!(comp.kind, NodeKind::Component { .. }) || doc.is_ancestor(inst, component) || doc.is_ancestor(component, inst) {
        // A missing main, or an instance inside its own main (would recurse forever).
        return;
    }
    let mut eff_props = default_props(&comp);
    for (k, v) in root_props {
        if eff_props.contains_key(k) {
            eff_props.insert(k.clone(), v.clone());
        }
    }
    // Old layers (kept alive for their overrides), by (parent, source), by source alone (when only
    // one layer mirrors it) and, only when those don't match, by name path.
    let mut old_ids = old_layers(doc, inst);
    old_ids.extend(spare.keys().copied());
    old_ids.sort();
    old_ids.dedup();
    let mut old: BTreeMap<NodeId, std::sync::Arc<Node>> = spare;
    let mut by_source: BTreeMap<(NodeId, NodeId), NodeId> = BTreeMap::new();
    let mut by_src: BTreeMap<NodeId, Option<NodeId>> = BTreeMap::new();
    for i in &old_ids {
        let Some(n) = doc.node_arc(*i).or_else(|| old.get(i)).cloned() else { continue };
        if let Some(s) = n.source {
            if let Some(p) = n.parent {
                by_source.entry((p, s)).or_insert(*i);
            }
            by_src.entry(s).and_modify(|e| *e = None).or_insert(Some(*i));
        }
        old.insert(*i, n);
    }
    let mut by_path: Option<BTreeMap<String, NodeId>> = None;
    let mut main_paths: Option<BTreeMap<NodeId, String>> = None;
    // Ids for the copies: reuse the matching old layer's id (a layer the main moved to another
    // parent keeps its id and overrides).
    let src: Vec<NodeId> = doc.subtree(component).into_iter().filter(|i| *i != component).collect();
    let mut map: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    map.insert(component, inst);
    let mut used: BTreeSet<NodeId> = BTreeSet::new();
    // Slots holding the instance's own content (new ids), and main layers not copied: the
    // default content of those slots.
    let mut edited: BTreeSet<NodeId> = BTreeSet::new();
    let mut skipped: BTreeSet<NodeId> = BTreeSet::new();
    let mut fresh_ids = Vec::new();
    for s in &src {
        let Some(sn) = doc.node(*s) else { continue };
        if sn.parent.is_some_and(|p| skipped.contains(&p) || map.get(&p).is_some_and(|pn| edited.contains(pn))) {
            skipped.insert(*s);
            continue;
        }
        let parent_new = sn.parent.and_then(|p| map.get(&p).copied());
        let mut found = parent_new.and_then(|pn| by_source.get(&(pn, *s)).copied()).or_else(|| by_src.get(s).copied().flatten());
        if found.is_none() {
            let paths = by_path.get_or_insert_with(|| path_map(doc, inst, true).into_iter().map(|(k, v)| (v, k)).collect());
            let mains = main_paths.get_or_insert_with(|| path_map(doc, component, false));
            found = mains.get(s).and_then(|p| paths.get(p)).copied();
        }
        let nid = match found {
            Some(o) if used.insert(o) => o,
            _ => {
                fresh_ids.push(*s);
                continue;
            }
        };
        if let Some(m) = old.get(&nid).and_then(|o| variant_kept(doc, o, *s)) {
            old.insert(nid, std::sync::Arc::new(m));
        }
        if old.get(&nid).is_some_and(|o| holds_content(o)) && is_slot(doc, *s) {
            edited.insert(nid);
        }
        map.insert(*s, nid);
    }
    for c in root.children.clone() {
        let _ = doc.remove(c);
    }
    for s in fresh_ids {
        let nid = doc.alloc_id();
        map.insert(s, nid);
    }
    // Build the copies (pre-order: parents first).
    let mut enclosing: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    enclosing.insert(inst, vec![inst]);
    let mut inst_props: BTreeMap<NodeId, BTreeMap<String, Value>> = BTreeMap::new();
    inst_props.insert(inst, eff_props);
    let mut nested = Vec::new();
    let mut new_nodes = Vec::new();
    // Text layers showing other text or style than their main layer (to re-measure).
    let mut remeasure = Vec::new();
    for s in &src {
        let Some(orig) = doc.node(*s) else { continue };
        let Some(&nid) = map.get(s) else { continue };
        let Some(parent) = orig.parent.and_then(|p| map.get(&p).copied()) else { continue };
        let mut n = orig.clone();
        n.id = nid;
        n.parent = Some(parent);
        n.children = orig.children.iter().filter_map(|c| map.get(c).copied()).collect();
        n.source = Some(*s);
        n.overrides.clear();
        n.prop_refs.clear();
        let prev = old.get(&nid);
        // Annotations belong to the layer they were added to, not to its copies.
        n.annotations = prev.map(|o| o.annotations.clone()).unwrap_or_default();
        if let Some(o) = prev {
            carry_overrides(o, &mut n, Carry::Values);
            keep_laid_out(doc, orig, o, &mut n);
        }
        if !edited.contains(&nid) {
            // No longer a slot in the main: it mirrors the main again.
            n.overrides.remove(SLOT);
        }
        // Component properties: the main layer's bindings use this instance's values; layers of
        // nested instances use the nested instance's values (one level per source hop).
        let enc = enclosing.get(&parent).cloned().unwrap_or_default();
        let mut chain: Vec<&Node> = vec![orig];
        while chain.len() < enc.len() {
            let Some(next) = chain.last().and_then(|c| c.source).and_then(|x| doc.node(x)) else { break };
            chain.push(next);
        }
        for (j, level) in chain.iter().enumerate().rev() {
            let Some(props) = enc.get(j).and_then(|e| inst_props.get(e)) else { continue };
            for (field, prop) in &level.prop_refs {
                let g = field_group(field);
                if n.overrides.contains(g) || (j > 0 && orig.overrides.contains(g)) {
                    continue;
                }
                if let Some(val) = props.get(prop) {
                    apply_prop(doc, &mut n, field, val);
                }
            }
        }
        if let NodeKind::Instance { component: c, props } = &n.kind {
            inst_props.insert(nid, props.clone());
            let mut e = enc;
            e.push(nid);
            enclosing.insert(nid, e);
            if !matches!(orig.kind, NodeKind::Instance { component: oc, .. } if oc == *c) {
                nested.push(nid);
            }
        } else {
            enclosing.insert(nid, enc);
        }
        if let (NodeKind::Text(a), NodeKind::Text(b)) = (&orig.kind, &n.kind)
            && a != b
        {
            remeasure.push(nid);
        }
        new_nodes.push(n);
    }
    for n in new_nodes {
        let (id, parent) = (n.id, n.parent);
        if insert_linked(doc, n).is_err()
            && let Some(p) = parent.and_then(|p| doc.node_mut(p))
        {
            p.children.retain(|c| *c != id);
        }
    }
    // Root: follows the main except its own placement, name, size and overrides.
    let order: Vec<NodeId> = comp.children.iter().filter_map(|c| map.get(c).copied()).filter(|c| doc.contains(*c)).collect();
    let mut fresh = comp.clone();
    fresh.id = inst;
    fresh.parent = root.parent;
    fresh.children = order;
    fresh.transform = root.transform;
    fresh.kind = root.kind.clone();
    carry_prop_bindings(&root, &mut fresh);
    // A nested instance keeps mirroring its layer of the outer main.
    let nested_copy = inside_instance(doc, inst);
    fresh.source = if nested_copy { root.source } else { Some(component) };
    fresh.constraints = root.constraints;
    fresh.child_layout = root.child_layout.clone();
    fresh.interactions = root.interactions.clone();
    fresh.exports = root.exports.clone();
    fresh.prop_refs = root.prop_refs.clone();
    fresh.plugin_data = root.plugin_data.clone();
    fresh.annotations = root.annotations.clone();
    // Code links belong to the main component; instances read them from it.
    fresh.code_links.clear();
    // An instance's own dev resources (its main's are listed with them, not copied).
    fresh.dev_resources = root.dev_resources.clone();
    fresh.locked = root.locked;
    fresh.visible = root.visible;
    fresh.var_modes = root.var_modes.clone();
    fresh.is_mask = root.is_mask;
    fresh.exposed = root.exposed;
    fresh.guides = root.guides.clone();
    fresh.dev_status = root.dev_status;
    fresh.dev_changed = root.dev_changed;
    fresh.dev_note = root.dev_note.clone();
    fresh.constrain_proportions = root.constrain_proportions;
    // How the instance combines in a boolean group it sits in is its own.
    fresh.bool_op = root.bool_op;
    // A swapped nested instance keeps the name its layer was given in the outer main.
    let renamed = nested_copy
        && root.source.and_then(|s| doc.node(s)).is_some_and(|s| match &s.kind {
            NodeKind::Instance { component: c, .. } => s.name != default_instance_name(doc, *c),
            _ => false,
        });
    fresh.name = if renamed { root.name.clone() } else { default_instance_name(doc, component) };
    // Switched to another variant: its own overrides stay where both variants agreed.
    if let Some(prev) = root.source.and_then(|p| doc.node(p)).filter(|p| sibling_variants(doc, p.id, component)) {
        let differs = diff_groups(prev, &comp);
        root.overrides.retain(|g| g == "name" || !differs.contains(&g.as_str()));
    }
    carry_overrides(&root, &mut fresh, Carry::Root);
    if let Some(r) = doc.node_mut(inst) {
        *r = fresh;
    }
    // A resized instance: children follow their constraints (auto layout re-flows later).
    let (iw, ih) = doc.node(inst).map(|n| (n.width, n.height)).unwrap_or((comp.width, comp.height));
    if (iw - comp.width).abs() > 1e-9 || (ih - comp.height).abs() > 1e-9 {
        figcraft_layout::apply_constraints(doc, inst, comp.width, comp.height);
        remeasure = map.values().copied().collect();
    }
    // Geometry overrides of children win over constraints.
    for s in &src {
        let Some(nid) = map.get(s).copied() else { continue };
        let Some(o) = old.get(&nid) else { continue };
        // A frame that kept the size its own resizing gave it: its layers (copied at their main
        // layers' places) follow their constraints from its main layer's size.
        if o.overrides.contains("resizing")
            && !o.overrides.contains("size")
            && doc.node(nid).is_some_and(|n| n.kind.is_frame_like() && n.layout.is_none())
            && let Some((mw, mh)) = doc.node(*s).map(|m| (m.width, m.height))
        {
            figcraft_layout::apply_constraints(doc, nid, mw, mh);
        }
        if !o.overrides.contains("size") && !o.overrides.contains("position") {
            continue;
        }
        remeasure.push(nid);
        let Some((ow, oh)) = doc.node(nid).map(|n| (n.width, n.height)) else { continue };
        if o.overrides.contains("size") {
            // Through set_size so vector networks scale with the box.
            figcraft_layout::set_size(doc, nid, o.width, o.height);
            if doc.node(nid).is_some_and(|n| n.kind.is_frame_like() && n.layout.is_none()) {
                figcraft_layout::apply_constraints(doc, nid, ow, oh);
            }
        }
        if let Some(n) = doc.node_mut(nid) {
            carry_overrides(o, n, Carry::Geometry);
        }
    }
    // Text whose characters or style differ from the main re-measures.
    for nid in remeasure {
        if let Some(n) = doc.node(nid)
            && let Some((w, h)) = figcraft_text::auto_size(n)
            && ((w - n.width).abs() > 1e-6 || (h - n.height).abs() > 1e-6)
        {
            figcraft_layout::set_size(doc, nid, w, h);
        }
    }
    // Nested instances showing another main than the main's copy (swapped): rebuild from it,
    // with the old layers below them this rebuild didn't reuse.
    let unused: BTreeMap<NodeId, std::sync::Arc<Node>> = old.into_iter().filter(|(i, _)| !used.contains(i) && !doc.contains(*i)).collect();
    for n in nested {
        let below: BTreeMap<NodeId, std::sync::Arc<Node>> = unused
            .iter()
            .filter(|(_, x)| {
                let mut cur = x.parent;
                for _ in 0..=MAX_DEPTH * 4 {
                    match cur {
                        Some(p) if p == n => return true,
                        Some(p) => cur = unused.get(&p).and_then(|a| a.parent),
                        None => return false,
                    }
                }
                false
            })
            .map(|(i, x)| (*i, x.clone()))
            .collect();
        sync_instance_depth(doc, n, depth + 1, below);
    }
}

/// Whether main components `a` and `b` are two variants of one set.
fn sibling_variants(doc: &Document, a: NodeId, b: NodeId) -> bool {
    a != b && variant_set(doc, a).is_some_and(|s| variant_set(doc, b) == Some(s))
}

/// Old copy `o` matched by name to main layer `s` of another variant (the instance switched
/// variants): `o` without the overrides of properties the two variants' layers differ in (`None`
/// when it keeps them all). Text layers keep every override.
fn variant_kept(doc: &Document, o: &Node, s: NodeId) -> Option<Node> {
    let from = o.source.filter(|f| *f != s).and_then(|f| doc.node(f))?;
    if o.overrides.is_empty() || matches!(o.kind, NodeKind::Text(_)) {
        return None;
    }
    if !sibling_variants(doc, owning_component(doc, from.id)?, owning_component(doc, s)?) {
        return None;
    }
    let differs = diff_groups(from, doc.node(s)?);
    let mut m = o.clone();
    m.overrides.retain(|g| !differs.contains(&g.as_str()));
    (m.overrides.len() != o.overrides.len()).then_some(m)
}

/// Drop the overrides of instance `inst` but its text layers' (and its own size and the content of
/// its slots): what swapping it from the Assets panel keeps.
pub fn keep_text_overrides(doc: &mut Document, inst: NodeId) {
    for d in old_layers(doc, inst).into_iter().chain([inst]) {
        let Some(n) = doc.node(d) else { continue };
        let text = matches!(n.kind, NodeKind::Text(_));
        let keep = |g: &String| text || g == SLOT || (d == inst && g == "size");
        if !n.overrides.iter().all(keep)
            && let Some(m) = doc.node_mut(d)
        {
            m.overrides.retain(keep);
        }
    }
}

/// Where auto layout places and sizes a rebuilt copy `n` of main layer `orig`, keep the old
/// copy's `o` values (an instance's auto layout re-flows from its own content, so they usually
/// differ from the main's): unchanged layers then stay unchanged, and auto layout re-flows wherever
/// something that sizes or places layers did change.
fn keep_laid_out(doc: &Document, orig: &Node, o: &Node, n: &mut Node) {
    if o.parent != n.parent || o.overrides.contains("size") || o.overrides.contains("position") {
        return;
    }
    let flows = !n.child_layout.absolute && orig.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some());
    if flows {
        // The main's rotation and skew, the old place.
        let [a, b, c, d, _, _] = n.transform.as_coeffs();
        let [.., e, f] = o.transform.as_coeffs();
        n.transform = Affine::new([a, b, c, d, e, f]);
    }
    // Only layers whose shape is their box (a vector's points scale with it).
    let boxed = n.kind.is_frame_like() || matches!(n.kind, NodeKind::Text(_) | NodeKind::Rectangle | NodeKind::Ellipse { .. });
    let derived = |s: Sizing| boxed && ((s == Sizing::Hug && n.layout.is_some()) || (s == Sizing::Fill && flows));
    if derived(n.child_layout.sizing_h) {
        n.width = o.width;
    }
    if derived(n.child_layout.sizing_v) {
        n.height = o.height;
    }
}

/// The layers of instance `inst` a rebuild matches against (every layer below it but the content
/// of slots holding its own), ascending.
fn old_layers(doc: &Document, inst: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![inst];
    while let Some(id) = stack.pop() {
        let Some(n) = doc.node(id) else { continue };
        for c in &n.children {
            out.push(*c);
            if out.len() > 1_000_000 {
                break;
            }
            if doc.node(*c).is_some_and(|cn| !holds_content(cn)) {
                stack.push(*c);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Insert a node whose parent/children links are already set (used when rebuilding instances).
fn insert_linked(doc: &mut Document, n: Node) -> Result<(), DocError> {
    let parent = n.parent.ok_or_else(|| DocError::Invalid("no parent".into()))?;
    let id = n.id;
    let mut n = n;
    let kids = std::mem::take(&mut n.children);
    doc.insert(n, parent, None)?;
    if let Some(m) = doc.node_mut(id) {
        m.children = kids;
    }
    // The parent may already list us (its children were copied with ids): keep the first.
    if let Some(p) = doc.node_mut(parent) {
        let mut seen = BTreeSet::new();
        p.children.retain(|c| seen.insert(*c));
    }
    Ok(())
}

/// The property groups whose value layer `n` takes from the same variables as its main layer
/// `src` (they differ only where the two resolve in different variable modes).
pub fn variable_groups(n: &Node, src: &Node) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (k, v) in &n.bound {
        let Some(g) = binding_group(k) else { continue };
        if src.bound.get(k) == Some(v) && !out.contains(&g) {
            out.push(g);
        }
    }
    out
}

/// The override group a variable binding key feeds (`None` for component property and layout
/// grid bindings, which follow other rules).
fn binding_group(key: &str) -> Option<&'static str> {
    use crate::cmd::lists::{BoundList, list_binding};
    if key.starts_with(PROP_BINDING) {
        return None;
    }
    match list_binding(key).map(|(l, _, _)| l) {
        Some(BoundList::Fills) => Some("fills"),
        Some(BoundList::Strokes) => Some("strokes"),
        Some(BoundList::Effects) => Some("effects"),
        Some(BoundList::Grids) => None,
        None => Some(crate::cmd::props::override_group(key)).filter(|g| *g != "other"),
    }
}

/// The variable bindings of `n` that feed group `g`.
fn bindings_of<'a>(n: &'a Node, g: &'a str) -> impl Iterator<Item = (&'a String, &'a String)> + 'a {
    n.bound.iter().filter(move |(k, _)| binding_group(k) == Some(g))
}

/// Property groups that differ between two versions of a node.
pub fn diff_groups(a: &Node, b: &Node) -> Vec<&'static str> {
    let mut out = Vec::new();
    if a.fills != b.fills {
        out.push("fills");
    }
    if a.strokes != b.strokes || a.stroke != b.stroke {
        out.push("strokes");
    }
    if a.effects != b.effects {
        out.push("effects");
    }
    if a.opacity != b.opacity || a.blend != b.blend {
        out.push("opacity");
    }
    if a.visible != b.visible {
        out.push("visible");
    }
    if a.name != b.name {
        out.push("name");
    }
    if a.corners != b.corners {
        out.push("corners");
    }
    if (a.width - b.width).abs() > 1e-6 || (a.height - b.height).abs() > 1e-6 {
        out.push("size");
    }
    if resizing(a) != resizing(b) {
        out.push("resizing");
    }
    if a.transform != b.transform {
        out.push("position");
    }
    if let (NodeKind::Text(x), NodeKind::Text(y)) = (&a.kind, &b.kind) {
        if x.characters != y.characters {
            out.push("characters");
        }
        if text_style_differs(x, y) {
            out.push("textStyle");
        }
    }
    if let (Some(x), Some(y)) = (&a.layout, &b.layout) {
        if x.padding != y.padding {
            out.push("padding");
        }
        if x.gap != y.gap || x.cross_gap != y.cross_gap {
            out.push("gap");
        }
        if x.main_align != y.main_align || x.cross_align != y.cross_align || x.content_align != y.content_align {
            out.push("alignment");
        }
    }
    if a.grids != b.grids || a.styles.grid != b.styles.grid {
        out.push("grids");
    }
    if a.exports != b.exports {
        out.push("exports");
    }
    if a.interactions != b.interactions {
        out.push("interactions");
    }
    if let (NodeKind::Instance { component: x, props: px }, NodeKind::Instance { component: y, props: py }) = (&a.kind, &b.kind) {
        if x != y {
            out.push("component");
        }
        if px != py {
            out.push("props");
        }
    }
    out
}

/// The `resizing` group of a layer: how it sizes on each axis, and its min and max sizes.
fn resizing(n: &Node) -> (Sizing, Sizing, [Option<f64>; 4]) {
    let c = &n.child_layout;
    (c.sizing_h, c.sizing_v, [c.min_w, c.max_w, c.min_h, c.max_h])
}

/// Whether two texts differ in anything but their characters (the `textStyle` group).
fn text_style_differs(x: &TextData, y: &TextData) -> bool {
    x.style != y.style
        || x.runs != y.runs
        || x.auto_resize != y.auto_resize
        || x.align_h != y.align_h
        || x.align_v != y.align_v
        || x.paragraph_spacing != y.paragraph_spacing
        || x.paragraph_indent != y.paragraph_indent
        || x.list != y.list
        || x.paragraphs != y.paragraphs
        || x.list_spacing != y.list_spacing
        || x.hanging_punctuation != y.hanging_punctuation
        || x.hanging_list != y.hanging_list
        || x.max_lines != y.max_lines
        || x.features != y.features
}

/// Whether a geometry change of `id` comes from layout rather than the user: an ancestor was
/// resized in the same edit (constraints), the parent is an auto layout frame, or auto-sized text.
fn geometry_derived(doc: &Document, before: &Document, id: NodeId, group: &str) -> bool {
    for a in doc.ancestors(id) {
        let (Some(x), Some(y)) = (doc.node(a), before.node(a)) else { continue };
        if (x.width - y.width).abs() > 1e-6 || (x.height - y.height).abs() > 1e-6 {
            return true;
        }
    }
    let Some(n) = doc.node(id) else { return true };
    if group == "position" && n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some()) && !n.child_layout.absolute {
        return true;
    }
    if group == "size"
        && let NodeKind::Text(t) = &n.kind
    {
        let wchanged = before.node(id).is_some_and(|b| (b.width - n.width).abs() > 1e-6);
        return match t.auto_resize {
            TextAutoResize::WidthAndHeight => true,
            TextAutoResize::Height => !wchanged,
            _ => false,
        };
    }
    false
}

/// After an edit: record overrides inside instances, then rebuild instances whose main changed,
/// whose properties or main were switched, or whose overrides were reset.
pub fn sync_after_edit(doc: &mut Document, before: &Document, changed: &[NodeId], touched: &BTreeSet<NodeId>) {
    // 0. A slot whose layers were added, removed or reordered holds the instance's own content
    //    from now on; copies of instance layers placed elsewhere stop mirroring their main.
    for &id in changed {
        let (Some(n), Some(b)) = (doc.node(id), before.node(id)) else { continue };
        if n.children == b.children || owning_instance(doc, id).is_none() {
            continue;
        }
        if let Some(slot) = slot_scope(doc, id)
            && !slot_edited(doc, slot)
        {
            own_slot_content(doc, slot);
        }
    }
    for &id in changed {
        if !before.contains(id) {
            relink_copy(doc, id);
        }
    }
    let mut resync: BTreeSet<NodeId> = BTreeSet::new();
    // 1. Overrides for layers edited inside instances.
    for &id in changed {
        let (Some(n), Some(b)) = (doc.node(id), before.node(id)) else { continue };
        let Some(inst) = owning_instance(doc, id) else { continue };
        let top_root = inst == id && !inside_instance(doc, id);
        if top_root
            && let (NodeKind::Instance { component: c1, props: p1 }, NodeKind::Instance { component: c2, props: p2 }) = (&n.kind, &b.kind)
            && (c1 != c2 || p1 != p2)
        {
            // Switching the main or a property value of an instance isn't an override.
            resync.insert(id);
        }
        let raw = diff_groups(b, n);
        let mut groups = raw.clone();
        // An instance's own place, resizing, export settings and interactions are its own, not
        // overrides.
        if top_root {
            groups.retain(|g| !matches!(*g, "component" | "props" | "position" | "resizing" | "exports" | "interactions"));
        }
        if groups.iter().any(|g| matches!(*g, "size" | "position")) {
            groups.retain(|g| !matches!(*g, "size" | "position") || !geometry_derived(doc, before, id, g));
        }
        // A value equal to the main's isn't an override (setting it back clears the override).
        let mut cleared = Vec::new();
        if let Some(src) = n.source.and_then(|s| doc.node(s)) {
            let mut differs = diff_groups(src, n);
            if inst == id {
                differs.retain(|g| *g != "name");
                if n.name != default_instance_name(doc, src.id) {
                    differs.push("name");
                }
            }
            // A value from the same variable as the main's (resolved in this layer's modes) isn't an
            // override either.
            let from_variables = variable_groups(n, src);
            groups.retain(|g| differs.contains(g) && !from_variables.contains(g));
            // Binding another variable (or none) where the main binds one is an override.
            for (k, _) in b.bound.iter().chain(n.bound.iter()) {
                let Some(g) = binding_group(k) else { continue };
                let rebound = b.bound.get(k) != n.bound.get(k) && !bindings_of(n, g).eq(bindings_of(src, g));
                if rebound && !groups.contains(&g) {
                    groups.push(g);
                }
            }
            cleared =
                raw.iter().filter(|g| !matches!(**g, "size" | "position") && !differs.contains(g) && n.overrides.contains(**g)).copied().collect();
            if top_root {
                cleared.retain(|g| !matches!(*g, "component" | "props"));
            }
        }
        let lost = b.overrides.iter().any(|g| !n.overrides.contains(g));
        let nested_switch = !top_root && raw.iter().any(|g| matches!(*g, "component" | "props"));
        if (!groups.is_empty() || !cleared.is_empty())
            && let Some(m) = doc.node_mut(id)
        {
            for g in groups {
                m.overrides.insert(g.to_string());
            }
            for g in cleared {
                m.overrides.remove(g);
            }
        }
        if (lost || nested_switch)
            && let Some(r) = root_instance(doc, id)
        {
            resync.insert(r);
        }
    }
    // 2. Components whose layers changed, were added or removed. Value-only edits (paint, text,
    //    visibility, …) are carried to the copies directly; anything else rebuilds the instances.
    let mut comps: BTreeSet<NodeId> = BTreeSet::new();
    let mut patches: Vec<(NodeId, Vec<&'static str>)> = Vec::new();
    // Main layers that only gained new layers or lost some: their copies take the same change.
    let mut restructured: Vec<NodeId> = Vec::new();
    let mut covered: BTreeSet<NodeId> = BTreeSet::new();
    for &id in changed {
        if let (Some(b), Some(n)) = (before.node(id), doc.node(id))
            && b.children != n.children
            && owning_instance(doc, id).is_none()
            && owning_component(doc, id).is_some()
            && let Some(layers) = restructurable(doc, before, b, n)
        {
            restructured.push(id);
            covered.extend(layers);
        }
    }
    for &id in changed {
        if covered.contains(&id) || restructured.contains(&id) {
            continue;
        }
        let Some(c) = owning_component(doc, id) else { continue };
        let (Some(b), Some(n)) = (before.node(id), doc.node(id)) else {
            comps.insert(c);
            continue;
        };
        if b == n {
            continue;
        }
        match patchable(doc, b, n) {
            Some(groups) => patches.push((id, groups)),
            None => {
                comps.insert(c);
            }
        }
    }
    for &id in touched {
        if !doc.contains(id)
            && !covered.contains(&id)
            && let Some(c) = owning_component(before, id)
            && doc.contains(c)
        {
            comps.insert(c);
        }
    }
    for r in resync {
        if doc.contains(r) {
            sync_instance(doc, r);
            if let Some(c) = owning_component(doc, r) {
                comps.insert(c);
            }
        }
    }
    // A copy the patch can't follow exactly rebuilds instead (as does any other change).
    let patched = comps.is_empty()
        && !full_rebuild()
        && (restructured.is_empty() || patch_structure(doc, before, &restructured))
        && (patches.is_empty() || patch_copies(doc, &patches));
    #[cfg(test)]
    if patched && (!restructured.is_empty() || !patches.is_empty()) {
        PATCHED.with(|c| c.set(c.get() + 1));
    }
    if !patched {
        comps.extend(patches.iter().map(|(id, _)| *id).chain(restructured.iter().copied()).filter_map(|id| owning_component(doc, id)));
    }
    // 3. Instances of changed components follow; an instance inside another component makes that
    //    one change too.
    rebuild_instances(doc, &comps);
}

#[cfg(test)]
thread_local! {
    /// Tests: rebuild instances even for edits that could be patched (to compare both ways).
    pub(crate) static FULL_REBUILD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Tests: edits carried to the copies without rebuilding instances.
    pub(crate) static PATCHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn full_rebuild() -> bool {
    FULL_REBUILD.with(std::cell::Cell::get)
}

#[cfg(not(test))]
fn full_rebuild() -> bool {
    false
}

/// The property groups of a main-component layer edited from `b` to `n` when the edit changed
/// values only (paint, text, visibility, size, position, …) that copies take as they are: carried to them without
/// rebuilding the instances ([`patch_copies`]). `None` when anything else changed.
fn patchable(doc: &Document, b: &Node, n: &Node) -> Option<Vec<&'static str>> {
    const VALUES: [&str; 17] = [
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
        "position",
    ];
    if b.children != n.children || b.parent != n.parent || b.kind.type_name() != n.kind.type_name() {
        return None;
    }
    let groups = diff_groups(b, n);
    if groups.is_empty() || groups.iter().any(|g| !VALUES.contains(g)) {
        return None;
    }
    // Exactly those groups (and the layer's own overrides) changed.
    let mut probe = n.clone();
    for g in &groups {
        copy_group(b, &mut probe, g);
    }
    probe.overrides = b.overrides.clone();
    if probe != *b {
        return None;
    }
    // Values a component property sets (on this layer or a main it mirrors) come from instances.
    let mut cur = Some(n);
    for _ in 0..=MAX_DEPTH {
        let Some(x) = cur else { break };
        if x.prop_refs.keys().any(|f| groups.contains(&field_group(f))) {
            return None;
        }
        cur = x.source.and_then(|s| doc.node(s));
    }
    Some(groups)
}

/// Copies by the layer they mirror; nested instances swapped to a main also follow it.
fn copies_index(doc: &Document) -> BTreeMap<NodeId, Vec<NodeId>> {
    let mut copies: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for n in doc.nodes() {
        let Some(s) = n.source else { continue };
        copies.entry(s).or_default().push(n.id);
        if let (NodeKind::Instance { component, .. }, Some(NodeKind::Instance { component: other, .. })) = (&n.kind, doc.node(s).map(|x| &x.kind))
            && component != other
        {
            copies.entry(*component).or_default().push(n.id);
        }
    }
    copies
}

/// When main layer `b` → `n` only gained new plain layers (created in this edit: no instances,
/// components or property bindings in them) or lost layers that were deleted: those layers, which
/// [`patch_structure`] carries to the copies. `None` for anything else.
fn restructurable(doc: &Document, before: &Document, b: &Node, n: &Node) -> Option<Vec<NodeId>> {
    let mut probe = n.clone();
    probe.children = b.children.clone();
    probe.overrides = b.overrides.clone();
    if probe != *b {
        return None;
    }
    let mut layers = Vec::new();
    for c in n.children.iter().filter(|c| !b.children.contains(c)) {
        for d in doc.subtree(*c) {
            let x = doc.node(d)?;
            let plain = !matches!(x.kind, NodeKind::Instance { .. } | NodeKind::Component { .. } | NodeKind::ComponentSet { .. });
            if before.contains(d) || !plain || !x.prop_refs.is_empty() {
                return None;
            }
            layers.push(d);
        }
    }
    for c in b.children.iter().filter(|c| !n.children.contains(c)) {
        if doc.contains(*c) {
            return None;
        }
        layers.extend(before.subtree(*c));
    }
    Some(layers)
}

/// Carry the new and deleted layers of main layers `seeds` ([`restructurable`]) to every copy,
/// through copies of copies (slots holding an instance's own content keep it). Returns false
/// (leaving a partial patch for the caller to rebuild over) when a copy can't follow exactly.
fn patch_structure(doc: &mut Document, before: &Document, seeds: &[NodeId]) -> bool {
    let copies = copies_index(doc);
    let mut work: Vec<NodeId> = seeds.to_vec();
    let mut steps = 0usize;
    while let Some(p) = work.pop() {
        steps += 1;
        if steps > 10_000_000 {
            return false;
        }
        let Some(pn) = doc.node(p).cloned() else { continue };
        for c in copies.get(&p).cloned().unwrap_or_default() {
            let Some(cn) = doc.node(c).cloned() else { continue };
            if holds_content(&cn) {
                continue;
            }
            if let (NodeKind::Instance { component: a, .. }, NodeKind::Instance { component: b, .. }) = (&pn.kind, &cn.kind)
                && a != b
            {
                continue;
            }
            // The copy's layers by the layer of `p` they mirror.
            let mut mirrors: BTreeMap<NodeId, NodeId> = BTreeMap::new();
            for k in &cn.children {
                let Some(src) = doc.node(*k).and_then(|x| x.source) else { return false };
                if mirrors.insert(src, *k).is_some() {
                    return false;
                }
            }
            let mut order = Vec::with_capacity(pn.children.len());
            for pc in &pn.children {
                if let Some(k) = mirrors.remove(pc) {
                    order.push(k);
                    continue;
                }
                // A layer the copy lacks must be new in this edit, placed as in the main.
                let absolute = doc.node(*pc).is_some_and(|x| x.child_layout.absolute);
                if before.contains(*pc) || !child_follows_main(doc, c, absolute) {
                    return false;
                }
                match copy_layers(doc, *pc, c) {
                    Some(k) => order.push(k),
                    None => return false,
                }
            }
            // Copies of layers the main lost.
            for (src, k) in mirrors {
                if doc.contains(src) || doc.remove(k).is_err() {
                    return false;
                }
            }
            if doc.node(c).is_some_and(|x| x.children != order)
                && let Some(m) = doc.node_mut(c)
            {
                m.children = order;
            }
            work.push(c);
        }
    }
    true
}

/// Whether a layer placed in copy `parent` (absolutely positioned or not) takes its main layer's
/// place and size: nothing on the way up applies constraints ([`follows_main_geometry`]).
fn child_follows_main(doc: &Document, parent: NodeId, absolute: bool) -> bool {
    let Some(p) = doc.node(parent) else { return false };
    let constrains = p.layout.is_none() || absolute;
    let resized = sized_own(p)
        || match &p.kind {
            NodeKind::Instance { component, .. } => {
                doc.node(*component).is_none_or(|m| (m.width - p.width).abs() > 1e-9 || (m.height - p.height).abs() > 1e-9)
            }
            _ => false,
        };
    !(constrains && resized) && follows_main_geometry(doc, parent)
}

/// Copy main layer `src` (with its layers) into copy `parent` as layers mirroring them; returns the
/// new copy (the caller places it among `parent`'s layers).
fn copy_layers(doc: &mut Document, src: NodeId, parent: NodeId) -> Option<NodeId> {
    let sub = doc.subtree(src);
    let map: BTreeMap<NodeId, NodeId> = sub.iter().map(|d| (*d, doc.alloc_id())).collect();
    for d in &sub {
        let mut n = doc.node(*d)?.clone();
        n.id = *map.get(d)?;
        n.parent = if *d == src { Some(parent) } else { n.parent.and_then(|x| map.get(&x).copied()) };
        n.children = n.children.iter().filter_map(|k| map.get(k).copied()).collect();
        n.source = Some(*d);
        n.overrides.clear();
        n.prop_refs.clear();
        n.annotations.clear();
        insert_linked(doc, n).ok()?;
    }
    map.get(&src).copied()
}

/// Carry value-only edits of main-component layers (`seeds`: layer, groups) to every copy, through
/// copies of copies, except where a copy overrides the group. Returns false (leaving a partial
/// patch for the caller to rebuild over) when a copy can't follow exactly.
fn patch_copies(doc: &mut Document, seeds: &[(NodeId, Vec<&'static str>)]) -> bool {
    let copies = copies_index(doc);
    let text = |g: &str| matches!(g, "characters" | "textStyle");
    let mut work: Vec<(NodeId, Vec<&'static str>)> = seeds.to_vec();
    let mut steps = 0usize;
    while let Some((p, groups)) = work.pop() {
        steps += 1;
        if steps > 10_000_000 {
            return false;
        }
        let Some(pn) = doc.node(p).cloned() else { continue };
        for c in copies.get(&p).cloned().unwrap_or_default() {
            let Some(cn) = doc.node(c) else { continue };
            // A swapped nested instance shows its own main, not the layer it stands in for.
            if let (NodeKind::Instance { component: a, .. }, NodeKind::Instance { component: b, .. }) = (&pn.kind, &cn.kind)
                && a != b
            {
                continue;
            }
            // An instance keeps its own visibility, place, export settings and interactions; its
            // name follows the main's unless renamed; a resized one lays its layers out against
            // the main's size.
            let root = matches!(pn.kind, NodeKind::Component { .. });
            if root && ((groups.contains(&"name") && !cn.overrides.contains("name")) || (groups.contains(&"size") && cn.overrides.contains("size"))) {
                return false;
            }
            let gs: Vec<&'static str> = groups
                .iter()
                .copied()
                .filter(|g| !cn.overrides.contains(*g) && !(root && matches!(*g, "visible" | "name" | "position" | "exports" | "interactions")))
                .collect();
            if gs.is_empty() {
                continue;
            }
            let geometry = gs.iter().any(|g| matches!(*g, "size" | "position"));
            if geometry && !follows_main_geometry(doc, c) {
                return false;
            }
            // Text content and style share their runs: a copy overriding one keeps its runs when the
            // other follows the main (as a rebuild carries them).
            let runs = match &cn.kind {
                NodeKind::Text(t) if gs.iter().any(|g| text(g)) && cn.overrides.iter().any(|g| text(g)) => Some(t.runs.clone()),
                _ => None,
            };
            if let Some(m) = doc.node_mut(c) {
                for g in &gs {
                    copy_group(&pn, m, g);
                }
                if let (Some(r), NodeKind::Text(t)) = (runs, &mut m.kind) {
                    t.runs = r;
                }
            }
            if (geometry || gs.iter().any(|g| text(g)))
                && let Some((w, h)) = doc.node(c).and_then(figcraft_text::auto_size)
                && doc.node(c).is_some_and(|n| (n.width - w).abs() > 1e-6 || (n.height - h).abs() > 1e-6)
            {
                figcraft_layout::set_size(doc, c, w, h);
            }
            work.push((c, gs));
        }
    }
    true
}

/// Whether a copy's size and place are its main layer's: no layer around it inside its instances
/// applies constraints on the way down to it (a resized instance or frame without auto layout, or
/// a resized auto layout instance holding it absolutely positioned). Auto layout re-flows the rest.
fn follows_main_geometry(doc: &Document, id: NodeId) -> bool {
    let chain = scope(doc, id);
    chain.windows(2).all(|w| {
        let (Some(below), Some(a)) = (w.first().and_then(|i| doc.node(*i)), w.get(1).and_then(|i| doc.node(*i))) else { return false };
        let constrains = a.layout.is_none() || below.child_layout.absolute;
        if sized_own(a) && constrains {
            return false;
        }
        match &a.kind {
            NodeKind::Instance { component, .. } => {
                !constrains || doc.node(*component).is_some_and(|m| (m.width - a.width).abs() <= 1e-9 && (m.height - a.height).abs() <= 1e-9)
            }
            _ => true,
        }
    })
}

/// Whether a layer inside an instance overrides its size or resizing: it may not have its main
/// layer's size, so the layers in it can't take their main layers' places as they are.
fn sized_own(n: &Node) -> bool {
    n.overrides.contains("size") || n.overrides.contains("resizing")
}

/// Rebuild every instance of `comps` (and of the components that hold those instances, and so on
/// outwards), each outermost instance once, inner components first.
fn rebuild_instances(doc: &mut Document, comps: &BTreeSet<NodeId>) {
    if comps.is_empty() {
        return;
    }
    // Outermost instances by the components their layers instantiate.
    let mut roots_of: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();
    for n in doc.nodes() {
        if let NodeKind::Instance { component, .. } = &n.kind {
            roots_of.entry(*component).or_default().insert(root_instance(doc, n.id).unwrap_or(n.id));
        }
    }
    // Affected components, with how deep their dependencies go (an instance inside component X of
    // component Y: X follows Y).
    let mut level: BTreeMap<NodeId, usize> = comps.iter().map(|c| (*c, 0)).collect();
    let mut outer: BTreeMap<NodeId, Option<NodeId>> = BTreeMap::new();
    let mut frontier: Vec<NodeId> = comps.iter().copied().collect();
    let mut rounds = 0;
    while !frontier.is_empty() && rounds < 64 {
        rounds += 1;
        let mut next = Vec::new();
        for c in frontier {
            let lc = level.get(&c).copied().unwrap_or(0);
            for r in roots_of.get(&c).into_iter().flatten() {
                let o = *outer.entry(*r).or_insert_with(|| owning_component(doc, *r));
                if let Some(x) = o
                    && x != c
                {
                    let lx = level.entry(x).or_insert(0);
                    if *lx < lc + 1 {
                        *lx = lc + 1;
                        next.push(x);
                    }
                }
            }
        }
        frontier = next;
    }
    // Instances inside components by level, then the rest (each once).
    let mut order: Vec<(usize, NodeId)> = outer.iter().map(|(r, o)| (o.and_then(|x| level.get(&x).copied()).unwrap_or(usize::MAX), *r)).collect();
    order.sort();
    for (_, r) in order {
        if doc.contains(r) {
            sync_instance(doc, r);
        }
    }
}

/// Slot `slot` (inside an instance) takes the instance's own content: the layers it shows (the
/// main's default content until now) stop mirroring the main, and instances among them stand
/// alone, linked to their own mains.
fn own_slot_content(doc: &mut Document, slot: NodeId) {
    if let Some(n) = doc.node_mut(slot) {
        n.overrides.insert(SLOT.into());
    }
    for nested in unlink_layers(doc, slot, None) {
        rebase_instance(doc, nested);
    }
}

/// Unlink the layers below `top` that are layers of `owner` (an instance, or `None` for the
/// content of a slot): plain layers forget the main layer they mirrored and their property
/// bindings. The instances among them are returned, to re-link to their own mains (with
/// [`rebase_instance`]) once the caller is done.
pub fn unlink_layers(doc: &mut Document, top: NodeId, owner: Option<NodeId>) -> Vec<NodeId> {
    let picked: Vec<(NodeId, bool)> =
        doc.subtree(top).into_iter().filter(|d| *d != top && enclosing_instance(doc, *d) == owner).map(|d| (d, is_instance(doc, d))).collect();
    let mut nested = Vec::new();
    for (d, instance) in picked {
        let Some(n) = doc.node_mut(d) else { continue };
        n.prop_refs.clear();
        if instance {
            nested.push(d);
        } else {
            n.source = None;
            n.overrides.clear();
        }
    }
    nested
}

/// A new layer copied from inside an instance (pasted, duplicated) to where no instance owns it
/// stops mirroring the main; a copied nested instance is re-linked to its own main (its look kept).
fn relink_copy(doc: &mut Document, id: NodeId) {
    let Some(n) = doc.node(id) else { return };
    match &n.kind {
        NodeKind::Instance { component, .. } => {
            let main_ok = doc.node(*component).is_some_and(|m| matches!(m.kind, NodeKind::Component { .. }));
            if main_ok && !inside_instance(doc, id) && (n.source != Some(*component) || !instance_issues(doc, id).is_empty()) {
                rebase_instance(doc, id);
            }
        }
        _ => {
            if (n.source.is_some() || !n.overrides.is_empty())
                && owning_instance(doc, id).is_none()
                && let Some(m) = doc.node_mut(id)
            {
                m.source = None;
                m.overrides.clear();
            }
        }
    }
}

/// Refuse edits that add, remove or reorder layers inside an instance (only its main can), outside
/// its slots.
pub fn check_structure_edit(before: &Document, after: &Document) -> Result<(), String> {
    for &id in after.dirty() {
        let (Some(n), Some(b)) = (after.node(id), before.node(id)) else { continue };
        // Library copies change only through library updates (and are read-only otherwise).
        if n.children == b.children || after.is_remote(id) {
            continue;
        }
        if structure_locked(before, id) && structure_locked(after, id) {
            let name = owning_instance(after, id).and_then(|i| after.node(i)).map(|i| i.name.clone()).unwrap_or_default();
            return Err(format!(
                "layers can't be added, removed or reordered inside the instance \"{name}\": edit its main component, or detach the instance first"
            ));
        }
    }
    Ok(())
}

/// Refuse edits an instance doesn't take as overrides ([`refused_override`]); the edit is rolled
/// back.
pub fn check_override_edit(before: &Document, after: &Document) -> Result<(), String> {
    match after.dirty().iter().find_map(|id| refused_override(before, after, *id)) {
        Some(what) => Err(format!("{what} comes from the main component in an instance: edit the main component, or detach the instance")),
        None => Ok(()),
    }
}

/// What about layer `id` an edit from `before` to `after` changed that only its main can: the
/// auto layout of an instance or a layer in one but its padding, gap and alignment, or the
/// position (rotation too), constraints or place in auto layout (absolute, grid cell) of a layer
/// inside an instance (its size and resizing are overrides). `None` when the edit is fine, also
/// when the main changed the same way in this edit (a library update).
pub fn refused_override(before: &Document, after: &Document, id: NodeId) -> Option<&'static str> {
    let (n, b) = (after.node(id)?, before.node(id)?);
    if after.is_remote(id) || n.source != b.source || owning_instance(before, id).is_none() || owning_instance(after, id).is_none() {
        return None;
    }
    let (mb, ma) = (before.node(n.source?)?, after.node(n.source?)?);
    let mode = |x: &Node| x.layout.as_ref().map(|l| l.mode);
    // Auto layout settings other than the overridable ones.
    let settings = |x: &Node| {
        x.layout.clone().map(|mut l| {
            (l.gap, l.cross_gap, l.padding, l.main_align, l.cross_align) = (0.0, 0.0, [0.0; 4], MainAlign::Min, CrossAlign::Min);
            l.content_align = ContentAlign::Auto;
            l
        })
    };
    // Child layout settings other than the resizing ones.
    let placing = |x: &Node| {
        let mut c = x.child_layout.clone();
        (c.sizing_h, c.sizing_v, c.min_w, c.max_w, c.min_h, c.max_h) = (Sizing::Fixed, Sizing::Fixed, None, None, None, None);
        c
    };
    if mode(n) != mode(b) && mode(ma) == mode(mb) {
        return Some("The auto layout direction");
    }
    if settings(n) != settings(b) && settings(ma) == settings(mb) {
        return Some("This auto layout setting");
    }
    if !inside_instance(after, id) || !inside_instance(before, id) {
        return None;
    }
    if n.transform != b.transform && ma.transform == mb.transform && !geometry_derived(after, before, id, "position") {
        return Some("A layer's position");
    }
    if n.constraints != b.constraints && ma.constraints == mb.constraints {
        return Some("A layer's constraints");
    }
    if placing(n) != placing(b) && placing(ma) == placing(mb) {
        return Some("A layer's place in auto layout");
    }
    None
}

/// Bounded source and parent lookups while a rebase reads an unchanged document.
struct RebaseSources<'a> {
    doc: &'a Document,
    chains: BTreeMap<NodeId, Option<Vec<NodeId>>>,
    parents: BTreeMap<(NodeId, NodeId), Option<bool>>,
    main_changes: BTreeMap<NodeId, bool>,
}

impl<'a> RebaseSources<'a> {
    const PARENTS: usize = 512;
    const CACHE: usize = 8192;

    fn new(doc: &'a Document) -> Self {
        Self { doc, chains: BTreeMap::new(), parents: BTreeMap::new(), main_changes: BTreeMap::new() }
    }

    fn chain(&mut self, source: NodeId) -> Option<Vec<NodeId>> {
        if let Some(chain) = self.chains.get(&source) {
            return chain.clone();
        }
        let read = || {
            let mut chain = Vec::new();
            let mut seen = BTreeSet::new();
            let mut current = Some(source);
            for _ in 0..MAX_DEPTH {
                let Some(id) = current else { return Some(chain) };
                if !seen.insert(id) {
                    return None;
                }
                let n = self.doc.node(id)?;
                chain.push(id);
                current = n.source;
            }
            current.is_none().then_some(chain)
        };
        let chain = read();
        if self.chains.len() < Self::CACHE {
            self.chains.insert(source, chain.clone());
        }
        chain
    }

    /// Validate the complete parent chain, even if `main` was found before a cycle.
    fn in_main(&mut self, id: NodeId, main: NodeId) -> Option<bool> {
        if let Some(found) = self.parents.get(&(id, main)) {
            return *found;
        }
        let read = || {
            let mut current = Some(id);
            let mut seen = BTreeSet::new();
            let mut found = false;
            for _ in 0..Self::PARENTS {
                let Some(id) = current else { return Some(found) };
                if !seen.insert(id) {
                    return None;
                }
                found |= id == main;
                current = self.doc.node(id)?.parent;
            }
            current.is_none().then_some(found)
        };
        let found = read();
        if self.parents.len() < Self::CACHE {
            self.parents.insert((id, main), found);
        }
        found
    }

    fn member(&mut self, source: NodeId, main: NodeId, target: NodeId) -> Option<NodeId> {
        let chain = self.chain(source)?;
        if chain.contains(&target) {
            return None;
        }
        let mut member = None;
        for id in chain {
            if self.in_main(id, main)? && id != main && member.is_none() {
                member = Some(id);
            }
        }
        member
    }

    /// Nearest enclosing instances and their current mains. Slots with own content end scope.
    fn owners(&self, id: NodeId) -> Option<Vec<(NodeId, NodeId)>> {
        let mut owners = Vec::new();
        let mut current = self.doc.node(id)?.parent;
        let mut seen = BTreeSet::from([id]);
        for _ in 0..Self::PARENTS {
            let Some(id) = current else { return Some(owners) };
            if !seen.insert(id) {
                return None;
            }
            let n = self.doc.node(id)?;
            if holds_content(n) {
                return Some(owners);
            }
            if let NodeKind::Instance { component, .. } = &n.kind {
                owners.push((id, *component));
            }
            current = n.parent;
        }
        current.is_none().then_some(owners)
    }

    fn contextual(&mut self, source: NodeId, owners: &[(NodeId, NodeId)]) -> bool {
        owners.iter().any(|(_, main)| self.in_main(source, *main) == Some(true) && source != *main)
    }

    /// A contextual child belongs to the same placed slot as its enclosing instance.
    fn valid(&mut self, source: NodeId, target: NodeId, owners: &[(NodeId, NodeId)]) -> bool {
        let Some((owner, main)) = owners.first() else { return false };
        if self.member(source, *main, target).is_none() {
            return false;
        }
        if self.in_main(source, *main) == Some(true) {
            return true;
        }
        let Some(source_owner) = self.doc.node(*owner).and_then(|n| n.source) else { return false };
        matches!(self.doc.node(source_owner).map(|n| &n.kind), Some(NodeKind::Instance { component, .. }) if component == main)
            && self.owners(source).and_then(|owners| owners.first().copied()).is_some_and(|(id, _)| id == source_owner)
    }

    fn placed(&mut self, id: NodeId, owners: &[(NodeId, NodeId)]) -> Option<NodeId> {
        let source = self.doc.node(id)?.source?;
        if !matches!(self.doc.node(source)?.kind, NodeKind::Instance { .. } | NodeKind::Frame)
            || !self.valid(source, id, owners)
            || !self.contextual(source, owners)
        {
            return None;
        }
        Some(source)
    }

    fn beneath_swap(&mut self, owners: &[(NodeId, NodeId)], stop: NodeId) -> bool {
        for (index, (id, main)) in owners.iter().enumerate() {
            if *id == stop {
                break;
            }
            let changed = if let Some(changed) = self.main_changes.get(id) {
                *changed
            } else {
                let changed = self.placed(*id, &owners[index + 1..]).is_some_and(|source| match self.doc.node(source).map(|n| &n.kind) {
                    Some(NodeKind::Instance { component, .. }) => component != main,
                    Some(NodeKind::Frame) => true,
                    _ => false,
                });
                if self.main_changes.len() < Self::CACHE {
                    self.main_changes.insert(*id, changed);
                }
                changed
            };
            if changed {
                return true;
            }
        }
        false
    }
}

/// Re-derive `source` links and overrides of instance `inst` against its own main by comparing
/// values (after detaching an outer instance, restoring a main, or copying a nested instance out):
/// the look is kept exactly. Values a component property sets aren't overrides.
pub fn rebase_instance(doc: &mut Document, inst: NodeId) {
    let Some(NodeKind::Instance { component, .. }) = doc.node(inst).map(|n| n.kind.clone()) else { return };
    let mine = path_map(doc, inst, true);
    let theirs: BTreeMap<String, NodeId> =
        if doc.contains(component) { path_map(doc, component, false).into_iter().map(|(k, v)| (v, k)).collect() } else { BTreeMap::new() };
    let value_groups = |a: &Node, b: &Node| -> Vec<String> {
        diff_groups(a, b).into_iter().filter(|g| !matches!(*g, "size" | "position")).map(str::to_string).collect()
    };
    let mut updates = Vec::new();
    let mut sources = RebaseSources::new(doc);
    let root_owners = sources.owners(inst).unwrap_or_default();
    let placed_source = sources.placed(inst, &root_owners);
    let mut contexts = vec![(inst, component)];
    contexts.extend_from_slice(&root_owners);
    for (id, path) in &mine {
        let Some(n) = doc.node(*id) else { continue };
        let owners = sources.owners(*id).unwrap_or_default();
        let nearest_main = owners.first().map(|(_, main)| *main);
        let valid_source = n.source.filter(|source| sources.valid(*source, *id, &owners));
        let mut main = valid_source.and_then(|source| sources.member(source, component, *id));
        // A swapped or Frame-to-Instance subtree is compared to its current main by the inner
        // rebase. An outer name path must not rebind those children to the old expanded copies.
        if main.is_none() && valid_source.is_some() && sources.beneath_swap(&owners, inst) {
            updates.push((*id, valid_source, n.overrides.clone()));
            continue;
        }
        main = main
            .or_else(|| theirs.get(path).copied().filter(|source| nearest_main.is_some_and(|scope| sources.member(*source, scope, *id).is_some())));
        let source = valid_source.filter(|source| sources.contextual(*source, &contexts)).filter(|_| main.is_some()).or(main);
        let mut overrides: BTreeSet<String> = n.overrides.iter().filter(|g| !matches!(g.as_str(), "component" | "props")).cloned().collect();
        if let Some(m) = source.and_then(|source| doc.node(source)) {
            let mut bound: Vec<&str> = m.prop_refs.keys().map(|f| field_group(f)).collect();
            bound.extend(variable_groups(n, m));
            overrides.extend(value_groups(m, n).into_iter().filter(|g| !bound.contains(&g.as_str())));
            if matches!((&m.kind, &n.kind), (NodeKind::Instance { component: a, .. }, NodeKind::Instance { component: b, .. }) if a != b) {
                overrides.insert("component".into());
            }
        }
        updates.push((*id, source, overrides));
    }
    // A placed nested root compares to its actual slot. A detached root compares to its own main.
    let root_base = placed_source.or_else(|| {
        let chain = sources.chain(component)?;
        (!chain.contains(&inst) && sources.in_main(component, inst) == Some(false)).then_some(component)
    });
    let root_groups = match (root_base.and_then(|source| doc.node(source)), doc.node(inst)) {
        (Some(m), Some(n)) => {
            let from_variables = variable_groups(n, m);
            let mut groups: Vec<String> = value_groups(m, n)
                .into_iter()
                .filter(|g| !matches!(g.as_str(), "name" | "exports" | "interactions") && !from_variables.contains(&g.as_str()))
                .filter(|g| placed_source.is_some() || g != "resizing")
                .collect();
            let named = if placed_source.is_some() { n.name != m.name } else { n.name != default_instance_name(doc, component) };
            if named {
                groups.push("name".into());
            }
            if placed_source.is_some()
                && matches!((&m.kind, &n.kind), (NodeKind::Instance { component: a, .. }, NodeKind::Instance { component: b, .. }) if a != b)
            {
                groups.push("component".into());
            }
            groups
        }
        _ => Vec::new(),
    };
    drop(sources);
    for (id, main, overrides) in updates {
        if let Some(n) = doc.node_mut(id) {
            n.source = main;
            n.overrides = if main.is_some() { overrides } else { BTreeSet::new() };
        }
    }
    if let Some(n) = doc.node_mut(inst) {
        n.source = root_base;
        // A top-level instance's resizing is its own (a nested one's overrides its main layer's).
        n.overrides.retain(|g| !matches!(g.as_str(), "component" | "props" | "position") && (placed_source.is_some() || g != "resizing"));
        n.overrides.extend(root_groups);
    }
}

#[cfg(test)]
#[path = "components_rebase_source_tests.rs"]
mod rebase_source_tests;

/// A set's default variant: the one in its top-left corner (nearest to it, then the top-most, then
/// the left-most).
pub fn default_variant(doc: &Document, set: NodeId) -> Option<NodeId> {
    let corner = |c: &NodeId| {
        doc.node(*c).filter(|n| matches!(n.kind, NodeKind::Component { .. })).map(|n| n.transform.as_coeffs()).map(|[.., x, y]| (x + y, y, x))
    };
    let kids = &doc.node(set)?.children;
    kids.iter()
        .copied()
        .filter_map(|c| corner(&c).map(|k| (k, c)))
        .min_by(|(a, _), (b, _)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, c)| c)
}

/// Variants of a set sharing one combination of property values (each group two or more, in
/// layer order): a conflict to resolve by giving each a unique combination.
pub fn variant_conflicts(doc: &Document, set: NodeId) -> Vec<Vec<NodeId>> {
    let mut by: BTreeMap<BTreeMap<String, String>, Vec<NodeId>> = BTreeMap::new();
    for c in doc.node(set).map(|s| s.children.clone()).unwrap_or_default() {
        if let Some(n) = doc.node(c).filter(|n| matches!(n.kind, NodeKind::Component { .. })) {
            by.entry(variant_props(&n.name)).or_default().push(c);
        }
    }
    by.into_values().filter(|v| v.len() > 1).collect()
}

/// Parse a variant name "Size=Large, State=Hover" into pairs.
pub fn variant_props(name: &str) -> BTreeMap<String, String> {
    name.split(',')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Format variant pairs as a layer name, in the given property order (others after, sorted).
pub fn variant_name(pairs: &BTreeMap<String, String>, order: &[String]) -> String {
    let mut keys: Vec<&String> = order.iter().filter(|k| pairs.contains_key(*k)).collect();
    keys.extend(pairs.keys().filter(|k| !order.contains(k)));
    keys.iter().filter_map(|k| pairs.get(*k).map(|v| format!("{k}={v}"))).collect::<Vec<_>>().join(", ")
}

/// Variant properties of a set: property → values, in first-seen order.
pub fn variant_options(doc: &Document, set: NodeId) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for c in doc.node(set).map(|s| s.children.clone()).unwrap_or_default() {
        let Some(cn) = doc.node(c) else { continue };
        for kv in cn.name.split(',') {
            let Some((k, v)) = kv.split_once('=') else { continue };
            let (k, v) = (k.trim().to_string(), v.trim().to_string());
            match out.iter_mut().find(|(p, _)| *p == k) {
                Some((_, vals)) => {
                    if !vals.contains(&v) {
                        vals.push(v);
                    }
                }
                None => out.push((k, vec![v])),
            }
        }
    }
    out
}

/// The variant in `set` matching `wanted` best: all given properties equal, or else the most.
pub fn find_variant(doc: &Document, set: NodeId, wanted: &BTreeMap<String, String>) -> Option<NodeId> {
    let s = doc.node(set)?;
    let score = |c: &NodeId| doc.node(*c).map(|n| variant_props(&n.name)).map(|p| wanted.iter().filter(|(k, v)| p.get(*k) == Some(*v)).count());
    if let Some(exact) = s.children.iter().copied().find(|c| score(c) == Some(wanted.len())) {
        return Some(exact);
    }
    s.children.iter().copied().filter(|c| score(c).is_some()).max_by_key(|c| score(c).unwrap_or(0))
}

/// Check that every instance mirrors its main's layer structure (tests and diagnostics): each
/// layer's `source` is the main layer at the same place, swapped nested instances mirror their
/// own main, slots holding the instance's own content aren't compared. Returns the problems found.
pub fn mirror_issues(doc: &Document) -> Vec<String> {
    doc.nodes()
        .filter(|n| matches!(n.kind, NodeKind::Instance { .. }) && !inside_instance(doc, n.id))
        .flat_map(|n| instance_issues(doc, n.id))
        .collect()
}

/// [`mirror_issues`] for one instance.
fn instance_issues(doc: &Document, id: NodeId) -> Vec<String> {
    fn walk(doc: &Document, inst: &Node, main: &Node, depth: usize, out: &mut Vec<String>) {
        if depth > MAX_DEPTH || holds_content(inst) {
            return;
        }
        if inst.children.len() != main.children.len() {
            out.push(format!("{} has {} layers, its main {} has {}", inst.id, inst.children.len(), main.id, main.children.len()));
            return;
        }
        for (a, b) in inst.children.iter().zip(&main.children) {
            let (Some(an), Some(bn)) = (doc.node(*a), doc.node(*b)) else { continue };
            if an.source != Some(bn.id) {
                out.push(format!("{} mirrors {:?}, expected {}", an.id, an.source, bn.id));
                continue;
            }
            match (&an.kind, &bn.kind) {
                (NodeKind::Instance { component: x, .. }, NodeKind::Instance { component: y, .. }) if x != y => {
                    if let Some(m) = doc.node(*x) {
                        walk(doc, an, m, depth + 1, out);
                    }
                }
                _ => walk(doc, an, bn, depth + 1, out),
            }
        }
    }
    let mut out = Vec::new();
    if let Some(n) = doc.node(id)
        && let NodeKind::Instance { component, .. } = &n.kind
        && let Some(m) = doc.node(*component)
    {
        walk(doc, n, m, 0, &mut out);
    }
    out
}

#[cfg(test)]
#[path = "components_tests.rs"]
pub(crate) mod tests;

/// The nested instance copies inside instance `id` whose main-component counterpart is exposed
/// (top-most first; an exposed instance's own exposed nested instances follow it).
pub fn exposed_copies(doc: &Document, id: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = doc.node(id).map(|n| n.children.iter().rev().copied().collect()).unwrap_or_default();
    let mut seen = 0usize;
    while let Some(c) = stack.pop() {
        seen += 1;
        if seen > 100_000 {
            break;
        }
        let Some(n) = doc.node(c) else { continue };
        // A hidden exposed instance hides its properties too.
        let exposed =
            n.visible && matches!(n.kind, NodeKind::Instance { .. }) && n.source.and_then(|src| doc.node(src)).is_some_and(|src| src.exposed);
        if exposed {
            out.push(c);
        }
        // Only exposed nested instances pass their own exposed instances up.
        if !matches!(n.kind, NodeKind::Instance { .. }) || exposed {
            stack.extend(n.children.iter().rev().copied());
        }
    }
    out
}
