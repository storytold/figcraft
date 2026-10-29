use super::cycles::{contains_itself_error, would_contain_itself};
use super::diff::{binding_group, bindings_of};
use super::sync::{insert_linked, sync_instance_depth};
use super::{
    Affine, BTreeMap, ChildLayout, DocError, Document, Node, NodeId, NodeKind, Value, default_instance_name, default_props, holds_content, is_slot,
    slot_edited,
};

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
    if would_contain_itself(doc, component, parent) {
        return Err(DocError::Invalid(contains_itself_error(doc, component)));
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
    // Measurements belong to the layer they were placed on.
    root.measurements.clear();
    // The main's interactions play on its instances in prototypes; the instance's own start empty.
    root.interactions.clear();
    // So do the variable modes the main sets (see `Document::inherited_modes`).
    root.var_modes.clear();
    doc.insert(root, parent, index)?;
    sync_instance(doc, id);
    Ok(id)
}

/// Copy `id` within its own file under `parent` (at `index`), keeping its local transform. A file
/// has one main per component, so a main component's copy is an instance of it; a variant copied
/// into its set stays a new variant, and every other layer is deep-copied.
pub fn copy_in_file(doc: &mut Document, id: NodeId, parent: NodeId, index: Option<usize>) -> Result<NodeId, DocError> {
    let n = doc.get(id)?;
    let in_set = doc.node(parent).is_some_and(|p| matches!(p.kind, NodeKind::ComponentSet { .. }));
    if !matches!(n.kind, NodeKind::Component { .. }) || in_set {
        return doc.duplicate_into(id, parent, index);
    }
    let transform = n.transform;
    let instance = create_instance_at(doc, id, parent, 0.0, 0.0, index)?;
    doc.get_mut(instance)?.transform = transform;
    Ok(instance)
}

/// Which part of the carried overrides to apply.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Carry {
    /// Paint, text, visibility, … (everything but geometry).
    Values,
    /// Values plus size (instance roots: their size decides how children follow constraints).
    Root,
    /// Size and position (children, after constraints ran).
    Geometry,
}

/// Copy the overridden property groups of `from` onto `to`.
pub(super) fn carry_overrides(from: &Node, to: &mut Node, what: Carry) {
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

/// An instance switched from variant `was` to `new`: its resizing (sizing and min / max per axis)
/// that is still `was`'s follows `new`; what the instance set itself stays in `own`.
pub(super) fn follow_variant_resizing(own: &mut ChildLayout, was: &ChildLayout, new: &ChildLayout) {
    if own.sizing_h == was.sizing_h {
        own.sizing_h = new.sizing_h;
    }
    if own.sizing_v == was.sizing_v {
        own.sizing_v = new.sizing_v;
    }
    for (o, w, n) in [
        (&mut own.min_w, was.min_w, new.min_w),
        (&mut own.max_w, was.max_w, new.max_w),
        (&mut own.min_h, was.min_h, new.min_h),
        (&mut own.max_h, was.max_h, new.max_h),
    ] {
        if *o == w {
            *o = n;
        }
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
        "modes" => to.var_modes = from.var_modes.clone(),
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
pub(super) fn path_map(doc: &Document, root: NodeId, src_names: bool) -> BTreeMap<NodeId, String> {
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
pub(super) fn carry_prop_bindings(from: &Node, to: &mut Node) {
    to.bound.retain(|k, _| !k.starts_with(PROP_BINDING));
    to.bound.extend(from.bound.iter().filter(|(k, _)| k.starts_with(PROP_BINDING)).map(|(k, v)| (k.clone(), v.clone())));
}

/// The override group a property-bound field belongs to.
pub(super) fn field_group(field: &str) -> &'static str {
    match field {
        "visible" => "visible",
        "characters" => "characters",
        "mainComponent" => "component",
        _ => "",
    }
}

/// Apply one component-property value to a layer.
pub(super) fn apply_prop(doc: &Document, n: &mut Node, field: &str, val: &Value) {
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
