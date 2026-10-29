use super::props::{Owner, add_prop_for, delete_prop, prop_defs};
use super::{
    BTreeMap, BTreeSet, Document, EngineError, NodeId, NodeKind, OVERRIDE_GROUPS, PROP_BINDING, PropDef, Result, SLOT, Session, Value, bad, boolean,
    copy_group, create_instance_at, default_props, get, id_of, inside_instance, is_slot, json, keep_text_overrides, main_of, ok, owning_component,
    owning_instance, rebase_instance, req_id, req_num, req_targets, root_instance, slot_edited, slot_name, slots_of, string, targets, unlink_layers,
    variant_options,
};

pub(super) fn detach(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.detach";
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    let mut out = Vec::new();
    for i in ids {
        if !matches!(doc.get(i)?.kind, NodeKind::Instance { .. }) {
            continue;
        }
        if inside_instance(doc, i) {
            return Err(EngineError::Disabled(cmd.into(), "a nested instance can't be detached on its own: detach the outer instance first".into()));
        }
        // Own layers lose their link (slot content stays as it is); nested instances stay
        // instances of their own mains.
        let nested = unlink_layers(doc, i, Some(i));
        keep_main_modes(doc, i)?;
        let n = doc.get_mut(i)?;
        n.kind = NodeKind::Frame;
        n.source = None;
        n.overrides.clear();
        n.prop_refs.remove("mainComponent");
        // No properties left to drive.
        n.bound.retain(|k, _| !k.starts_with(PROP_BINDING));
        for nn in nested {
            rebase_instance(doc, nn);
        }
        out.push(i);
    }
    if out.is_empty() {
        return Err(EngineError::Disabled(cmd.into(), "select an instance".into()));
    }
    s.set_selection(out.clone());
    Ok(json!({"ids": out}))
}

/// Make the variable modes instance `i` takes from its main its own (it's being detached).
fn keep_main_modes(doc: &mut Document, i: NodeId) -> Result<()> {
    let modes = match doc.get(i)?.kind {
        NodeKind::Instance { component, .. } => doc.node(component).map(|m| m.var_modes.clone()).unwrap_or_default(),
        _ => return Ok(()),
    };
    let own = &mut doc.get_mut(i)?.var_modes;
    for (c, m) in modes {
        own.entry(c).or_insert(m);
    }
    Ok(())
}

pub(super) fn reset_instance(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.resetInstance";
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    let mut done = Vec::new();
    for i in ids {
        let Some(inst) = owning_instance(doc, i) else { continue };
        let top = !inside_instance(doc, inst);
        for d in doc.subtree(inst) {
            let keep: Vec<&str> = if d == inst && top { vec!["size"] } else { vec![] };
            if let Some(n) = doc.node(d)
                && n.overrides.iter().any(|g| !keep.contains(&g.as_str()))
                && let Some(n) = doc.node_mut(d)
            {
                n.overrides.retain(|g| keep.contains(&g.as_str()));
            }
        }
        // Property values go back to the defaults.
        if top
            && let Some(NodeKind::Instance { component, props }) = doc.node(inst).map(|n| n.kind.clone())
            && let Some(defaults) = doc.node(component).map(default_props)
            && props != defaults
            && let NodeKind::Instance { props, .. } = &mut doc.get_mut(inst)?.kind
        {
            *props = defaults;
        }
        // ...and stop following variables.
        if top && doc.node(inst).is_some_and(|n| n.bound.keys().any(|k| k.starts_with(PROP_BINDING))) {
            doc.get_mut(inst)?.bound.retain(|k, _| !k.starts_with(PROP_BINDING));
        }
        done.push(inst);
    }
    if done.is_empty() {
        return Err(EngineError::Disabled(cmd.into(), "select an instance".into()));
    }
    Ok(json!({"ids": done}))
}

pub(super) fn reset_override(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.resetOverride";
    let i = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a layer inside an instance"))?;
    let group = string(p, "group");
    if let Some(g) = &group
        && !OVERRIDE_GROUPS.contains(&g.as_str())
    {
        return Err(bad(cmd, format!("unknown group `{g}` (known: {})", OVERRIDE_GROUPS.join(", "))));
    }
    let doc = s.doc_mut()?;
    if owning_instance(doc, i).is_none() || doc.get(i)?.source.is_none() {
        return Err(bad(cmd, "not a layer of an instance"));
    }
    let top_root = matches!(doc.get(i)?.kind, NodeKind::Instance { .. }) && !inside_instance(doc, i);
    let had = doc.get(i)?.overrides.clone();
    let n = doc.get_mut(i)?;
    match &group {
        Some(g) => {
            n.overrides.remove(g);
        }
        None => n.overrides.clear(),
    }
    let mut changed = had != doc.get(i)?.overrides;
    // The instance's own property values reset with `props`.
    if top_root
        && group.as_deref().is_none_or(|g| g == "props")
        && let Some(NodeKind::Instance { component, props }) = doc.node(i).map(|n| n.kind.clone())
        && let Some(defaults) = doc.node(component).map(default_props)
        && props != defaults
        && let NodeKind::Instance { props, .. } = &mut doc.get_mut(i)?.kind
    {
        *props = defaults;
        changed = true;
    }
    Ok(json!({"changed": changed}))
}

/// A property name for `owner` based on `base` (a layer name): valid and not taken yet.
fn fresh_prop_name(doc: &Document, owner: &Owner, base: &str) -> String {
    let base: String = base.chars().filter(|c| !matches!(c, ',' | '=')).take(100).collect::<String>().trim().to_string();
    let base = if base.is_empty() { "Slot".to_string() } else { base };
    let taken = |n: &str| {
        owner.components(doc).iter().any(|c| prop_defs(doc, *c).contains_key(n))
            || matches!(owner, Owner::Set(s) if variant_options(doc, *s).iter().any(|(k, _)| k == n))
    };
    (1..10_000).map(|k| if k == 1 { base.clone() } else { format!("{base} {k}") }).find(|n| !taken(n)).unwrap_or(base)
}

/// The `preferred` components of a slot property: component or component set ids.
pub(super) fn preferred_arg(cmd: &str, doc: &Document, p: &Value) -> Result<Vec<NodeId>> {
    let Some(v) = get(p, "preferred").filter(|v| !v.is_null()) else { return Ok(Vec::new()) };
    let list = v.as_array().ok_or_else(|| bad(cmd, "`preferred`: an array of component ids"))?;
    let mut out = Vec::new();
    for x in list.iter().take(500) {
        let c = id_of(x)
            .filter(|c| doc.node(*c).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. })))
            .ok_or_else(|| bad(cmd, format!("`preferred`: {x} is not a component or component set")))?;
        if !out.contains(&c) {
            out.push(c);
        }
    }
    Ok(out)
}

pub(super) fn make_slot(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.makeSlot";
    let id = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a frame inside a main component"))?;
    let doc = s.doc()?;
    let n = doc.get(id)?;
    let inside_main = owning_component(doc, id).is_some_and(|c| c != id) && owning_instance(doc, id).is_none();
    if !matches!(n.kind, NodeKind::Frame) || !inside_main {
        return Err(bad(cmd, "only a frame inside a main component (not inside an instance) can become a slot"));
    }
    if is_slot(doc, id) {
        return Err(EngineError::Disabled(cmd.into(), format!("\"{}\" is already a slot", n.name)));
    }
    let owner = Owner::resolve(doc, id).ok_or_else(|| bad(cmd, "the frame isn't inside a component"))?;
    let name = string(p, "name").unwrap_or_else(|| fresh_prop_name(doc, &owner, &n.name));
    let preferred = get(p, "preferred").cloned().unwrap_or(Value::Null);
    add_prop_for(cmd, s, &json!({"id": id, "layer": id, "name": name, "type": "slot", "preferred": preferred}))
}

pub(super) fn remove_slot(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.removeSlot";
    let id = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a slot frame of a main component"))?;
    let doc = s.doc()?;
    if owning_instance(doc, id).is_some() {
        return Err(EngineError::Disabled(cmd.into(), "slots are removed on the main component".into()));
    }
    let name = slot_name(doc, id).ok_or_else(|| bad(cmd, "not a slot frame of a main component"))?;
    delete_prop(s, &json!({"id": id, "name": name}))?;
    Ok(json!({"id": id, "name": name}))
}

/// The slot inside an instance a command acts on: `id` itself, or instance `id`'s slot `name`.
fn instance_slot(cmd: &str, doc: &Document, id: NodeId, name: Option<&str>) -> Result<NodeId> {
    let slot = match name {
        Some(nm) => slots_of(doc, id).into_iter().find(|(_, n)| n == nm).map(|(l, _)| l).ok_or_else(|| bad(cmd, format!("no slot named `{nm}`")))?,
        None => id,
    };
    if !is_slot(doc, slot) || owning_instance(doc, slot).is_none() {
        return Err(bad(cmd, "not a slot of an instance (a main component's slot takes layers like any frame)"));
    }
    Ok(slot)
}

pub(super) fn slot_insert(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "instance.slotInsert";
    let c = req_id(cmd, p, "component")?;
    let target = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a slot inside an instance"))?;
    let index = match get(p, "index").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => Some(v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(cmd, "`index`: a child index (0 = bottom)"))?),
    };
    let doc = s.doc_mut()?;
    let slot = instance_slot(cmd, doc, target, string(p, "slot").as_deref())?;
    let c = main_of(doc, c).ok_or_else(|| bad(cmd, "`component` must be a component or component set"))?;
    if boolean(p, "replace").unwrap_or(false) {
        for k in doc.get(slot)?.children.clone() {
            doc.remove(k)?;
        }
    }
    let i = create_instance_at(doc, c, slot, 0.0, 0.0, index)?;
    s.set_selection(vec![i]);
    Ok(json!({"id": i, "slot": slot}))
}

pub(super) fn reset_slot(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "instance.resetSlot";
    let id = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a slot inside an instance"))?;
    let doc = s.doc_mut()?;
    let slot = instance_slot(cmd, doc, id, string(p, "slot").as_deref())?;
    let changed = slot_edited(doc, slot);
    if changed {
        doc.get_mut(slot)?.overrides.remove(SLOT);
    }
    Ok(json!({"id": slot, "changed": changed}))
}

pub(super) fn push_changes(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.pushChanges";
    let i = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select an instance"))?;
    let doc = s.doc_mut()?;
    // Changes go to the outer instance's main (never into a nested instance's own main).
    let inst = root_instance(doc, i).ok_or_else(|| bad(cmd, "not an instance"))?;
    let Some(NodeKind::Instance { component, .. }) = doc.node(inst).map(|n| n.kind.clone()) else { return ok() };
    if !doc.contains(component) {
        return Err(EngineError::Disabled(cmd.into(), "the main component was deleted (use component.restoreMain)".into()));
    }
    // Copy every overridden layer's values to its source in the main (geometry, resizing, swaps,
    // property values and slot content stay with the instance).
    let mut pushed = 0;
    for d in doc.subtree(inst) {
        let Some(n) = doc.node(d).cloned() else { continue };
        let Some(src) = (if d == inst { Some(component) } else { n.source }) else { continue };
        if n.overrides.is_empty() || !doc.contains(src) || !doc.is_ancestor(component, src) {
            continue;
        }
        let Some(mut target) = doc.node(src).cloned() else { continue };
        let groups: BTreeSet<String> = n
            .overrides
            .iter()
            .filter(|g| !(matches!(g.as_str(), "size" | "resizing" | "position" | "component" | "props" | SLOT) || d == inst && g.as_str() == "name"))
            .cloned()
            .collect();
        for g in &groups {
            copy_group(&n, &mut target, g);
        }
        if groups.is_empty() {
            continue;
        }
        pushed += 1;
        if let Some(slot) = doc.node_mut(src) {
            *slot = target;
        }
        if let Some(m) = doc.node_mut(d) {
            m.overrides.retain(|g| !groups.contains(g));
        }
    }
    Ok(json!({"layers": pushed}))
}

fn restore_prop_defs(doc: &Document, props: &BTreeMap<String, Value>) -> BTreeMap<String, PropDef> {
    let mut defs = BTreeMap::new();
    for (k, v) in props {
        let def = match v {
            Value::Bool(b) => PropDef::Boolean { default: *b },
            Value::String(t) => PropDef::Text { default: t.clone() },
            other => match id_of(other) {
                Some(c) if doc.contains(c) => PropDef::InstanceSwap { default: c, preferred: Vec::new() },
                _ => continue,
            },
        };
        defs.insert(k.clone(), def);
    }
    defs
}

fn restore_id_map(doc: &mut Document, i: NodeId, component: NodeId, sub: &[NodeId]) -> BTreeMap<NodeId, NodeId> {
    let mut map: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    let mut used: BTreeSet<NodeId> = BTreeSet::new();
    map.insert(i, component);
    used.insert(component);
    for d in sub.iter().filter(|d| **d != i) {
        let src = doc.node(*d).and_then(|n| n.source);
        let nid = match src {
            Some(s) if !doc.contains(s) && used.insert(s) => s,
            _ => doc.alloc_id(),
        };
        map.insert(*d, nid);
    }
    map
}

pub(super) fn restore_main(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.restoreMain";
    let i = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select an instance"))?;
    let doc = s.doc_mut()?;
    let Some(NodeKind::Instance { component, props }) = doc.node(i).map(|n| n.kind.clone()) else {
        return Err(bad(cmd, "select an instance"));
    };
    if doc.contains(component) {
        return Err(EngineError::Disabled(cmd.into(), "the main component still exists".into()));
    }
    if inside_instance(doc, i) {
        return Err(EngineError::Disabled(cmd.into(), "select the outer instance".into()));
    }
    let parent = doc.get(i)?.parent.ok_or_else(|| bad(cmd, "no parent"))?;
    let index = doc.index_in_parent(i);
    // The main comes back with its old id (other instances point to it) and its layers with the
    // ids their copies mirror, so every instance re-links by source.
    let sub = doc.subtree(i);
    let map = restore_id_map(doc, i, component, &sub);
    let mut nodes = Vec::new();
    let mut nested = Vec::new();
    for d in &sub {
        let Some(mut n) = doc.node(*d).cloned() else { continue };
        if *d == i {
            let defs = restore_prop_defs(doc, &props);
            n.kind = NodeKind::Component { description: String::new(), props: defs, prop_order: Vec::new() };
            n.source = None;
            n.overrides.clear();
            n.prop_refs.clear();
        } else {
            let parent_inst = n.parent.and_then(|p| owning_instance(doc, p));
            if parent_inst != Some(i) {
                nodes.push(n);
                continue;
            }
            if matches!(n.kind, NodeKind::Instance { .. }) {
                nested.push(*d);
            } else {
                n.source = None;
                n.overrides.clear();
            }
        }
        nodes.push(n);
    }
    doc.remove(i)?;
    doc.paste_nodes(&nodes, &map, i, parent, index)?;
    for nn in nested {
        if let Some(m) = map.get(&nn).copied() {
            rebase_instance(doc, m);
        }
    }
    s.set_selection(vec![component]);
    Ok(json!({"id": component}))
}

pub(super) fn swap(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "instance.swap";
    let c = req_id(cmd, p, "component")?;
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    let c = main_of(doc, c).ok_or_else(|| bad(cmd, "`component` must be a component or component set"))?;
    for i in ids {
        swap_instance(doc, i, c).map_err(|m| bad(cmd, m))?;
    }
    ok()
}

/// Make instance `i` show main component `c`, keeping the values of properties with the same names
/// (the layers' overrides follow by name when the instance is rebuilt).
pub(crate) fn swap_instance(doc: &mut Document, i: NodeId, c: NodeId) -> std::result::Result<(), String> {
    if crate::components::would_contain_itself(doc, c, i) {
        return Err(crate::components::contains_itself_error(doc, c));
    }
    let defaults = doc.node(c).map(default_props).unwrap_or_default();
    match doc.get_mut(i).map(|n| &mut n.kind) {
        Ok(NodeKind::Instance { component, props }) => {
            *component = c;
            let old = std::mem::take(props);
            *props = defaults.iter().map(|(k, d)| (k.clone(), old.get(k).cloned().unwrap_or_else(|| d.clone()))).collect();
            Ok(())
        }
        _ => Err(format!("{i} is not an instance")),
    }
}

/// Swap the instance under a page point to a component (`instance.swapAt`).
pub(crate) fn swap_at(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "instance.swapAt";
    let c = req_id(cmd, p, "component")?;
    let pt = figcraft_geom::Point::new(req_num(cmd, p, "x")?, req_num(cmd, p, "y")?);
    let page = s.page()?;
    let doc = s.doc()?;
    let c = main_of(doc, c).ok_or_else(|| bad(cmd, "`component` must be a component or component set"))?;
    let family = crate::drop::swap_family(doc, c);
    let target = crate::drop::swap_target(doc, page, pt, family.as_deref(), Some(c))
        .ok_or_else(|| EngineError::Disabled(cmd.into(), "there is no instance at that point to swap".into()))?;
    keep_text_overrides(s.doc_mut()?, target);
    swap(s, &json!({"ids": [target], "component": c}))?;
    s.set_selection(vec![target]);
    Ok(json!({"id": target, "component": c}))
}
