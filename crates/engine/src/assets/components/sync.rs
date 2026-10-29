use super::cycles::expansion_layers;
use super::diff::diff_groups;
use super::instances::{Carry, apply_prop, carry_overrides, carry_prop_bindings, copy_group, field_group, follow_variant_resizing, path_map};
use super::{
    Affine, BTreeMap, BTreeSet, DocError, Document, MAX_DEPTH, Node, NodeId, NodeKind, SLOT, Sizing, Value, default_instance_name, default_props,
    holds_content, inside_instance, is_slot, owning_component, variant_set,
};

/// `spare`: layers the outer rebuild didn't reuse that were below `inst` (a swapped nested
/// instance): its layers the outer main doesn't have, matched here so they keep ids and overrides.
pub(super) fn sync_instance_depth(doc: &mut Document, inst: NodeId, depth: usize, spare: BTreeMap<NodeId, std::sync::Arc<Node>>) {
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
    let eff_props = effective_props(&comp, root_props);
    // Old layers (kept alive for their overrides), by (parent, source), by source alone (when only
    // one layer mirrors it) and, only when those don't match, by name path.
    let (mut old, by_source, by_src) = index_old_layers(doc, inst, spare);
    // Never a copy of itself inside it (components containing each other).
    let Some(src) = expansion_layers(doc, inst, component) else { return };
    let Matched { mut map, used, edited, fresh } = match_layers(doc, inst, component, &src, &mut old, &by_source, &by_src);
    for c in root.children.clone() {
        let _ = doc.remove(c);
    }
    for s in fresh {
        let nid = doc.alloc_id();
        map.insert(s, nid);
    }
    let Copies { nodes: new_nodes, nested, mut remeasure } = copy_layers(doc, inst, &src, &map, &old, &edited, eff_props);
    for n in new_nodes {
        let (id, parent) = (n.id, n.parent);
        if insert_linked(doc, n).is_err()
            && let Some(p) = parent.and_then(|p| doc.node_mut(p))
        {
            p.children.retain(|c| *c != id);
        }
    }
    let fresh = fresh_root(doc, &mut root, &comp, inst, component, &map);
    if let Some(r) = doc.node_mut(inst) {
        *r = fresh;
    }
    // A resized instance: children follow their constraints (auto layout re-flows later).
    let (iw, ih) = doc.node(inst).map(|n| (n.width, n.height)).unwrap_or((comp.width, comp.height));
    if (iw - comp.width).abs() > 1e-9 || (ih - comp.height).abs() > 1e-9 {
        figcraft_layout::apply_constraints(doc, inst, comp.width, comp.height);
        remeasure = map.values().copied().collect();
    }
    follow_geometry_overrides(doc, &src, &map, &old, &mut remeasure);
    remeasure_text(doc, remeasure);
    rebuild_nested(doc, nested, &old, &used, depth);
}

/// Which instance layer each main layer a rebuild copies becomes.
struct Matched {
    /// Main layer → its copy's id (the matching old layer's, so a layer the main moved to another
    /// parent keeps its id and overrides); the main itself → the instance.
    map: BTreeMap<NodeId, NodeId>,
    /// The old layers reused.
    used: BTreeSet<NodeId>,
    /// Slots holding the instance's own content (their main layers' default content isn't copied).
    edited: BTreeSet<NodeId>,
    /// Main layers no old layer matches: their copies get new ids.
    fresh: Vec<NodeId>,
}

/// Match the main layers `src` of instance `inst`'s main `component` to its old layers: by
/// (parent, source), by source alone (when only one layer mirrors it) and, only when those don't
/// match, by name path. Old layers matched across variants drop what the two variants differ in.
fn match_layers(
    doc: &Document,
    inst: NodeId,
    component: NodeId,
    src: &[NodeId],
    old: &mut BTreeMap<NodeId, std::sync::Arc<Node>>,
    by_source: &BTreeMap<(NodeId, NodeId), NodeId>,
    by_src: &BTreeMap<NodeId, Option<NodeId>>,
) -> Matched {
    let mut by_path: Option<BTreeMap<String, NodeId>> = None;
    let mut main_paths: Option<BTreeMap<NodeId, String>> = None;
    let mut m = Matched { map: BTreeMap::from([(component, inst)]), used: BTreeSet::new(), edited: BTreeSet::new(), fresh: Vec::new() };
    let mut skipped: BTreeSet<NodeId> = BTreeSet::new();
    for s in src {
        let Some(sn) = doc.node(*s) else { continue };
        if sn.parent.is_some_and(|p| skipped.contains(&p) || m.map.get(&p).is_some_and(|pn| m.edited.contains(pn))) {
            skipped.insert(*s);
            continue;
        }
        let parent_new = sn.parent.and_then(|p| m.map.get(&p).copied());
        let mut found = parent_new.and_then(|pn| by_source.get(&(pn, *s)).copied()).or_else(|| by_src.get(s).copied().flatten());
        if found.is_none() {
            let paths = by_path.get_or_insert_with(|| path_map(doc, inst, true).into_iter().map(|(k, v)| (v, k)).collect());
            let mains = main_paths.get_or_insert_with(|| path_map(doc, component, false));
            found = mains.get(s).and_then(|p| paths.get(p)).copied();
        }
        let nid = match found {
            Some(o) if m.used.insert(o) => o,
            _ => {
                m.fresh.push(*s);
                continue;
            }
        };
        if let Some(kept) = old.get(&nid).and_then(|o| variant_kept(doc, o, *s)) {
            old.insert(nid, std::sync::Arc::new(kept));
        }
        if old.get(&nid).is_some_and(|o| holds_content(o)) && is_slot(doc, *s) {
            m.edited.insert(nid);
        }
        m.map.insert(*s, nid);
    }
    m
}

/// A rebuild's copies of the main's layers, pre-order (parents first).
struct Copies {
    nodes: Vec<Node>,
    /// Nested instances showing another main than their main layer (swapped): rebuilt from it.
    nested: Vec<NodeId>,
    /// Text layers showing other text or style than their main layer (to re-measure).
    remeasure: Vec<NodeId>,
}

/// Copy main layers `src` into instance `inst` as `map` names them, with the overrides of the
/// `old` layers they replace and the component property values (`props`: the instance's).
fn copy_layers(
    doc: &Document,
    inst: NodeId,
    src: &[NodeId],
    map: &BTreeMap<NodeId, NodeId>,
    old: &BTreeMap<NodeId, std::sync::Arc<Node>>,
    edited: &BTreeSet<NodeId>,
    props: BTreeMap<String, Value>,
) -> Copies {
    let mut enclosing: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::from([(inst, vec![inst])]);
    let mut inst_props: BTreeMap<NodeId, BTreeMap<String, Value>> = BTreeMap::from([(inst, props)]);
    let mut out = Copies { nodes: Vec::new(), nested: Vec::new(), remeasure: Vec::new() };
    for s in src {
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
        n.measurements = prev.map(|o| o.measurements.clone()).unwrap_or_default();
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
        apply_enclosing_props(doc, &mut n, orig, &enc, &inst_props);
        if let NodeKind::Instance { component: c, props } = &n.kind {
            inst_props.insert(nid, props.clone());
            let mut e = enc;
            e.push(nid);
            enclosing.insert(nid, e);
            if !matches!(orig.kind, NodeKind::Instance { component: oc, .. } if oc == *c) {
                out.nested.push(nid);
            }
        } else {
            enclosing.insert(nid, enc);
        }
        if let (NodeKind::Text(a), NodeKind::Text(b)) = (&orig.kind, &n.kind)
            && a != b
        {
            out.remeasure.push(nid);
        }
        out.nodes.push(n);
    }
    out
}

/// Geometry overrides of children win over constraints.
fn follow_geometry_overrides(
    doc: &mut Document,
    src: &[NodeId],
    map: &BTreeMap<NodeId, NodeId>,
    old: &BTreeMap<NodeId, std::sync::Arc<Node>>,
    remeasure: &mut Vec<NodeId>,
) {
    for s in src {
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
}

/// Nested instances showing another main than the main's copy (swapped): rebuild from it,
/// with the old layers below them this rebuild didn't reuse.
fn rebuild_nested(doc: &mut Document, nested: Vec<NodeId>, old: &BTreeMap<NodeId, std::sync::Arc<Node>>, used: &BTreeSet<NodeId>, depth: usize) {
    let unused: BTreeMap<NodeId, std::sync::Arc<Node>> =
        old.iter().filter(|(i, _)| !used.contains(*i) && !doc.contains(**i)).map(|(i, x)| (*i, x.clone())).collect();
    for n in nested {
        let mut below: BTreeMap<NodeId, std::sync::Arc<Node>> = unused
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
        take_placing_overrides(doc, n, old, &mut below);
        sync_instance_depth(doc, n, depth + 1, below);
    }
}

fn apply_enclosing_props(doc: &Document, n: &mut Node, orig: &Node, enc: &[NodeId], inst_props: &BTreeMap<NodeId, BTreeMap<String, Value>>) {
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
                apply_prop(doc, n, field, val);
            }
        }
    }
}

/// Text whose characters or style differ from the main re-measures.
fn remeasure_text(doc: &mut Document, remeasure: Vec<NodeId>) {
    for nid in remeasure {
        if let Some(n) = doc.node(nid)
            && let Some((w, h)) = figcraft_text::auto_size(n)
            && ((w - n.width).abs() > 1e-6 || (h - n.height).abs() > 1e-6)
        {
            figcraft_layout::set_size(doc, nid, w, h);
        }
    }
}

/// The component's property defaults with the instance's values for the ones it has.
fn effective_props(comp: &Node, root_props: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    let mut eff_props = default_props(comp);
    for (k, v) in root_props {
        if eff_props.contains_key(k) {
            eff_props.insert(k.clone(), v.clone());
        }
    }
    eff_props
}

type OldIndex = (BTreeMap<NodeId, std::sync::Arc<Node>>, BTreeMap<(NodeId, NodeId), NodeId>, BTreeMap<NodeId, Option<NodeId>>);

/// The old layers of `inst` (and `spare`) by id, by (parent, source) and by source alone (`None`
/// when several layers mirror it).
fn index_old_layers(doc: &Document, inst: NodeId, spare: BTreeMap<NodeId, std::sync::Arc<Node>>) -> OldIndex {
    let mut old_ids = old_layers(doc, inst);
    old_ids.extend(spare.keys().copied());
    old_ids.sort();
    old_ids.dedup();
    let mut old = spare;
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
    (old, by_source, by_src)
}

/// The rebuilt root of instance `inst` of `comp`.
fn fresh_root(doc: &Document, root: &mut Node, comp: &Node, inst: NodeId, component: NodeId, map: &BTreeMap<NodeId, NodeId>) -> Node {
    // Root: follows the main except its own placement, name, size and overrides.
    let order: Vec<NodeId> = comp.children.iter().filter_map(|c| map.get(c).copied()).filter(|c| doc.contains(*c)).collect();
    let mut fresh = comp.clone();
    fresh.id = inst;
    fresh.parent = root.parent;
    fresh.children = order;
    fresh.transform = root.transform;
    fresh.kind = root.kind.clone();
    carry_prop_bindings(root, &mut fresh);
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
    fresh.measurements = root.measurements.clone();
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
        let differs = diff_groups(prev, comp);
        root.overrides.retain(|g| g == "name" || !differs.contains(&g.as_str()));
        follow_variant_resizing(&mut fresh.child_layout, &prev.child_layout, &comp.child_layout);
    }
    carry_overrides(root, &mut fresh, Carry::Root);
    fresh
}

/// Before swapped nested instance `n` is rebuilt from its new main: each of its old layers (the
/// copies just made of the outer main's, and `below`) takes the overrides held along the source
/// chain it had before this rebuild (`before`), through layers inside instances in the mains that
/// place `n`. Matched by name, they carry onto the new main's layers as the instance's own, so a
/// swap takes what the outer main set once: later rebuilds find layers mirroring the new main.
/// Geometry and slot content follow the new main; groups the layer overrides itself keep its value.
fn take_placing_overrides(
    doc: &mut Document,
    n: NodeId,
    before: &BTreeMap<NodeId, std::sync::Arc<Node>>,
    below: &mut BTreeMap<NodeId, std::sync::Arc<Node>>,
) {
    let placing = placing_mains(doc, n);
    for id in doc.subtree(n).into_iter().filter(|i| *i != n) {
        let held = before.get(&id).map(|b| held_along(doc, b, &placing)).unwrap_or_default();
        if let Some(layer) = doc.node_mut(id) {
            take(layer, &held);
        }
    }
    for layer in below.values_mut() {
        let held = held_along(doc, layer, &placing);
        if !held.is_empty() {
            take(std::sync::Arc::make_mut(layer), &held);
        }
    }
}

/// The mains placing nested instance `n`: those of the layers it mirrors, outward.
fn placing_mains(doc: &Document, n: NodeId) -> BTreeSet<NodeId> {
    let mut out = BTreeSet::new();
    let mut cur = doc.node(n).and_then(|x| x.source);
    for _ in 0..=MAX_DEPTH {
        let Some(s) = cur.and_then(|c| doc.node(c)).filter(|s| !matches!(s.kind, NodeKind::Component { .. })) else { break };
        out.extend(owning_component(doc, s.id));
        cur = s.source;
    }
    out
}

/// The overrides but geometry and slot content held along `layer`'s source chain while it runs
/// through layers inside instances in `placing` mains, each with the nearest layer holding it.
fn held_along(doc: &Document, layer: &Node, placing: &BTreeSet<NodeId>) -> BTreeMap<String, std::sync::Arc<Node>> {
    let mut out = BTreeMap::new();
    let mut cur = layer.source;
    for _ in 0..=MAX_DEPTH {
        let Some(s) = cur.and_then(|c| doc.node_arc(c)) else { break };
        if !inside_instance(doc, s.id) || !owning_component(doc, s.id).is_some_and(|m| placing.contains(&m)) {
            break;
        }
        for g in s.overrides.iter().filter(|g| !matches!(g.as_str(), "size" | "position" | SLOT)) {
            out.entry(g.clone()).or_insert_with(|| s.clone());
        }
        cur = s.source;
    }
    out
}

/// Give `layer` the `held` groups it has no override of its own for.
fn take(layer: &mut Node, held: &BTreeMap<String, std::sync::Arc<Node>>) {
    for (g, from) in held {
        if layer.overrides.insert(g.clone()) {
            copy_group(from, layer, g);
        }
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
pub(super) fn insert_linked(doc: &mut Document, n: Node) -> Result<(), DocError> {
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
