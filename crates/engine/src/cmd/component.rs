//! Component commands: components, variants, component properties and instances.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::*;
use serde_json::{Value, json};

use super::edit::{roots, z_sorted};
use super::{CommandSpec, ok};
use crate::components::*;
use crate::params::*;
use crate::{EngineError, Result, Session, bad, spec};

/// `component.hideWhenPublishing`: the leading `.` of the names of this file's components and
/// sets (those `ids` are or are in), put on or taken off.
fn hide_when_publishing(s: &mut Session, p: &Value) -> Result<Value> {
    use crate::library::{asset_root, hidden_name, with_hidden_name};
    let cmd = "component.hideWhenPublishing";
    let doc = s.doc()?;
    let mut ids: Vec<NodeId> = Vec::new();
    for i in req_targets(cmd, s, p)? {
        let root = asset_root(doc, i).ok_or_else(|| bad(cmd, format!("{i} isn't a component, a component set or a layer in one")))?;
        if doc.is_remote(root) {
            return Err(EngineError::Disabled(cmd.into(), crate::library::READ_ONLY.into()));
        }
        if !ids.contains(&root) {
            ids.push(root);
        }
    }
    let hidden = |doc: &Document, i: &NodeId| doc.node(*i).is_some_and(|n| hidden_name(&n.name));
    let on = boolean(p, "on").unwrap_or_else(|| !ids.iter().all(|i| hidden(doc, i)));
    let doc = s.doc_mut()?;
    for i in &ids {
        let n = doc.get_mut(*i)?;
        n.name = with_hidden_name(&n.name, on);
    }
    Ok(json!({"ids": ids, "hidden": on}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "component.create",
            "Create component",
            "{ids?}  (one frame/group becomes a component; several layers are wrapped in one)",
            true,
            create
        ),
        spec!("component.createMultiple", "Create multiple components", "{ids?}", true, |s, p| {
            let ids = req_targets("component.createMultiple", s, p)?;
            let mut out = Vec::new();
            for i in ids {
                let r = create(s, &json!({"ids": [i]}))?;
                out.push(r["id"].clone());
            }
            s.set_selection(out.iter().filter_map(id_of).collect());
            Ok(json!({"ids": out}))
        }),
        spec!(
            "component.combineVariants",
            "Combine as variants",
            "{ids? (main components; none already a variant, a library copy or inside an instance)}  (wraps them in a new component set; names without `=` become \"Property 1=Default\", \"Property 1=VariantN\")",
            true,
            combine
        ),
        spec!(
            "component.addVariant",
            "Add variant",
            "{id? (component set, variant, or a main component not in a set yet: it becomes a set of itself and the new variant), property?, value?}  (duplicates the variant; `property`=`value` names the new one)",
            true,
            add_variant
        ),
        spec!("instance.create", "Create instance", "{component (component or set: its default variant), x?, y?, parent?}", true, |s, p| {
            let cmd = "instance.create";
            let c = req_id(cmd, p, "component")?;
            let page = s.page()?;
            let parent = crate::params::opt_id(cmd, p, "parent")?.unwrap_or(page);
            let doc = s.doc_mut()?;
            let c = main_of(doc, c).ok_or_else(|| bad(cmd, "`component` must be a component or component set"))?;
            let (x, y) = match (num(p, "x"), num(p, "y")) {
                (Some(x), Some(y)) if x.is_finite() && y.is_finite() => (x, y),
                _ => doc.world_bounds(c).map(|b| (b.x1 + 40.0, b.y0)).unwrap_or((0.0, 0.0)),
            };
            let i = create_instance(doc, c, parent, x, y)?;
            s.set_selection(vec![i]);
            Ok(json!({"id": i}))
        }),
        spec!(
            "instance.place",
            "Place instance",
            "{component, x, y (page point: the instance is centred there, inside the frame under it; auto layout frames insert by position)}",
            true,
            place
        ),
        spec!(
            "component.exposeInstance",
            "Expose nested instance",
            "{id? (an instance inside a main component), exposed? (default true)} — its properties then show on the main's instances",
            true,
            |s, p| {
                let cmd = "component.exposeInstance";
                let id = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a nested instance"))?;
                let exposed = boolean(p, "exposed").unwrap_or(true);
                let doc = s.doc_mut()?;
                let ok_target = doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Instance { .. }))
                    && owning_component(doc, id).is_some_and(|c| c != id)
                    && !inside_instance(doc, id);
                if !ok_target {
                    return Err(bad(cmd, "only instances nested inside a main component can be exposed"));
                }
                doc.get_mut(id)?.exposed = exposed;
                Ok(json!({"id": id, "exposed": exposed}))
            }
        ),
        spec!(
            "instance.exposed",
            "Exposed nested instances",
            "{id? (instance)} → the nested instance copies whose properties it exposes",
            false,
            |s, p| {
                let id = targets(s, p).first().copied().ok_or_else(|| bad("instance.exposed", "select an instance"))?;
                let doc = s.doc()?;
                let out: Vec<serde_json::Value> =
                    exposed_copies(doc, id).into_iter().filter_map(|c| doc.node(c).map(|n| json!({"id": c, "name": n.name}))).collect();
                Ok(json!({"instances": out}))
            }
        ),
        spec!(
            "component.makeSlot",
            "Make slot",
            "{id? (a frame inside a main component), name? (the slot property's name; the frame's by default), preferred? [component or set ids]}  (its instances take any content there)",
            true,
            make_slot
        ),
        spec!(
            "component.removeSlot",
            "Remove slot",
            "{id? (a slot frame of a main component)}  (instances lose their own content in it)",
            true,
            remove_slot
        ),
        spec!(
            "instance.slotInsert",
            "Insert into slot",
            "{id? (a slot inside an instance, or the instance with `slot`), slot? (slot property name), component, index? (child index; on top when omitted), replace? (clear the slot first)}",
            true,
            slot_insert
        ),
        spec!(
            "instance.resetSlot",
            "Reset slot",
            "{id? (a slot inside an instance, or the instance with `slot`), slot?}  (back to the main's default content)",
            true,
            reset_slot
        ),
        spec!(
            "component.hideWhenPublishing",
            "Hide when publishing",
            "{ids? (components, sets or layers in them; default the selection), on? (default: toggle, hiding unless all are hidden)}  keeps them out of the library this file publishes, the way a name starting with `.` or `_` does: hiding puts a `.` before the name, showing takes the leading `.` and `_` away → {ids, hidden}",
            true,
            hide_when_publishing
        ),
        spec!("component.detach", "Detach instance", "{ids?}", true, detach),
        spec!("component.resetInstance", "Reset all changes", "{ids?}", true, reset_instance),
        spec!(
            "component.resetOverride",
            "Reset override",
            "{id?, group? (fills|strokes|effects|opacity|visible|name|corners|characters|textStyle|padding|gap|alignment|grids|exports|interactions|size|position|component|props|slot; all when omitted)}",
            true,
            reset_override
        ),
        spec!("component.goToMain", "Go to main component", "{id?}", false, |s, p| {
            let cmd = "component.goToMain";
            let i = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select an instance"))?;
            let doc = s.doc()?;
            let inst = owning_instance(doc, i).ok_or_else(|| bad(cmd, "not an instance"))?;
            let Some(NodeKind::Instance { component, .. }) = doc.node(inst).map(|n| n.kind.clone()) else {
                return Err(bad(cmd, "not an instance"));
            };
            if !doc.contains(component) {
                return Err(EngineError::Disabled(cmd.into(), "the main component was deleted (use component.restoreMain)".into()));
            }
            if let Some(lib) = crate::library::library_of(doc, component) {
                let name = crate::library::library_name(doc, lib);
                return Err(EngineError::Disabled(cmd.into(), format!("the main component is in the library \"{name}\": open that file to edit it")));
            }
            super::edit::select_and_page(s, vec![component])?;
            let ds = s.ds_mut()?;
            if let Some(b) = ds.doc.world_bounds(component) {
                ds.view.fit(b, 2.0);
            }
            Ok(json!({"id": component}))
        }),
        spec!("component.pushChanges", "Push changes to main component", "{id?}", true, push_changes),
        spec!(
            "component.restoreMain",
            "Restore main component",
            "{id?}  (an instance whose main was deleted becomes the main again, in its place)",
            true,
            restore_main
        ),
        spec!("instance.swap", "Swap instance", "{ids?, component}", true, swap),
        spec!(
            "instance.swapAt",
            "Swap instance at point",
            "{component (component or set: its default variant), x, y (page point)}  swaps the instance there (preferring one showing the same component set; Alt-drop from the Assets panel); only text layers keep their overrides (by matching names)",
            true,
            swap_at
        ),
        spec!(
            "instance.setProperties",
            "Set instance properties",
            "{ids?, properties {name: value}} (variant properties switch variants)",
            true,
            set_props
        ),
        spec!(
            "component.addProperty",
            "Add component property",
            "{id? (component, set, or a layer inside a component: it gets bound), name, type: boolean|text|instanceSwap|variant|slot, default?, layer? (node bound to it; a frame for slots), preferred? (instance swap and slot properties: component or set ids offered first)}",
            true,
            add_prop
        ),
        spec!(
            "component.editProperty",
            "Edit component property",
            "{id? (component or set), name, newName?, default? (variant properties: that variant takes the top-left place), value? + newValue? (rename a variant value), index? (variant properties: move to this place; the variants' names follow), preferred? (instance swap and slot properties: component or set ids offered first)}",
            true,
            edit_prop
        ),
        spec!("component.deleteProperty", "Delete component property", "{id? (component or set), name}", true, delete_prop),
        spec!(
            "component.bindProperty",
            "Bind layer to property",
            "{layer?, property (null/\"\" unbinds), field?: visible|characters|mainComponent|slot}",
            true,
            bind_prop
        ),
        spec!("component.properties", "Component properties", "{id? (component, set or instance)}", false, |s, p| {
            let i = targets(s, p).first().copied().ok_or_else(|| bad("component.properties", "select a component or instance"))?;
            Ok(describe_props(s.doc()?, i))
        }),
        spec!(
            "component.list",
            "List components",
            "{} → [{id, name, type, page, instances, folder (the Assets panel's folders: page, frames, slash names), label, library?}]",
            false,
            |s, _| {
                let doc = s.doc()?;
                Ok(Value::Array(
                doc.nodes()
                    .filter(|n| matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. }))
                    .map(|n| {
                        let (folder, label) = crate::library::asset_folder(doc, n.id);
                        let mut v = json!({"id": n.id, "name": n.name, "type": n.kind.type_name(), "page": doc.page_of(n.id), "instances": instances_of(doc, n.id).len(), "folder": folder, "label": label});
                        if let Some(lib) = crate::library::library_of(doc, n.id) {
                            v["library"] = json!(lib);
                            v["page"] = Value::Null;
                        }
                        v
                    })
                    .collect(),
            ))
            }
        ),
    ]
}

/// Place an instance of `component` centred on page point (`x`, `y`), in the frame or auto layout
/// slot under it (`instance.place`).
pub(crate) fn place(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "instance.place";
    let c = req_id(cmd, p, "component")?;
    let pt = figcraft_geom::Point::new(req_num(cmd, p, "x")?, req_num(cmd, p, "y")?);
    let page = s.page()?;
    let doc = s.doc_mut()?;
    let c = main_of(doc, c).ok_or_else(|| bad(cmd, "`component` must be a component or component set"))?;
    let mut slot = crate::drop::slot(doc, page, pt);
    // Never inside its own main component (or a copy of it).
    if doc.is_ancestor(c, slot.parent) || slot.parent == c {
        slot = crate::drop::DropSlot { parent: page, index: None, line: None };
    }
    let (w, h) = doc.node(c).map(|n| (n.width, n.height)).unwrap_or((0.0, 0.0));
    let local = doc.world_transform(slot.parent).inverse() * figcraft_geom::Point::new(pt.x - w / 2.0, pt.y - h / 2.0);
    let i = crate::components::create_instance_at(doc, c, slot.parent, local.x, local.y, slot.index)?;
    s.set_selection(vec![i]);
    Ok(json!({"id": i, "parent": slot.parent}))
}

/// A component to instantiate: the component itself, or a set's default (top-left) variant.
pub(crate) fn main_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    match doc.node(id)?.kind {
        NodeKind::Component { .. } => Some(id),
        NodeKind::ComponentSet { .. } => default_variant(doc, id),
        _ => None,
    }
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.create";
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    if ids.iter().any(|i| inside_instance(doc, *i)) {
        return Err(EngineError::Disabled(cmd.into(), "layers inside an instance can't become components".into()));
    }
    let ids = z_sorted(doc, &roots(doc, &ids));
    let target = if ids.len() == 1 && ids.first().and_then(|i| doc.node(*i)).is_some_and(|n| matches!(n.kind, NodeKind::Frame | NodeKind::Group)) {
        ids.first().copied().ok_or_else(|| bad(cmd, "nothing selected"))?
    } else {
        // Wrap the layers in a frame first (like Figma); a single layer lends it its name.
        let single_name = if let [only] = ids.as_slice() { doc.node(*only).map(|n| n.name.clone()).filter(|n| !n.is_empty()) } else { None };
        let r = super::structure::specs()
            .into_iter()
            .find(|x| x.id == "object.frameSelection")
            .map(|x| (x.run)(s, &json!({"ids": ids})))
            .ok_or_else(|| EngineError::Other("frame selection unavailable".into()))??;
        let fid = id_of(&r["id"]).ok_or_else(|| EngineError::Other("wrap failed".into()))?;
        let doc = s.doc_mut()?;
        let n = doc.get_mut(fid)?;
        n.fills.clear();
        if let Some(name) = single_name {
            n.name = name;
        }
        fid
    };
    let doc = s.doc_mut()?;
    let n = doc.get_mut(target)?;
    let was_group = matches!(n.kind, NodeKind::Group);
    n.kind = NodeKind::Component { description: String::new(), props: Default::default() };
    if was_group {
        n.fills.clear();
        n.clip = false;
        n.blend = figcraft_color::BlendMode::PassThrough;
    }
    s.set_selection(vec![target]);
    Ok(json!({"id": target}))
}

/// The name a new component set takes from its first variant: the path before the last '/'
/// ("Button/State=Hover" → "Button"), or a plain name as is ("Button"). A bare "prop=value" name
/// ("State=Default, Size=Small") names no set.
fn set_name_from(variant: &str) -> Option<String> {
    let base = variant.rsplit_once('/').map_or(variant, |(path, _)| path).trim();
    (!base.is_empty() && !base.contains('=')).then(|| base.to_string())
}

/// Wrap components in a new component set (named after the first) placed around them.
fn wrap_in_set(cmd: &str, doc: &mut Document, ids: &[NodeId]) -> Result<NodeId> {
    let top = *ids.last().ok_or_else(|| bad(cmd, "nothing selected"))?;
    let parent = doc.get(top)?.parent.ok_or_else(|| bad(cmd, "page"))?;
    if inside_instance(doc, top) || owning_instance(doc, parent).is_some() {
        return Err(EngineError::Disabled(cmd.into(), INSIDE_INSTANCE.into()));
    }
    let bounds = doc.bounds_of(ids).unwrap_or_default();
    let pw = doc.world_transform(parent);
    let local = figcraft_geom::transform_rect_bbox(pw.inverse(), bounds.inflate(20.0, 20.0));
    let base = ids.first().and_then(|i| doc.node(*i)).and_then(|n| set_name_from(&n.name)).unwrap_or_else(|| doc.next_name("Component"));
    let set = doc.alloc_id();
    let mut n = Node::new(set, NodeKind::ComponentSet { description: String::new() }, base, local.width(), local.height());
    n.transform = figcraft_geom::Affine::translate((local.x0, local.y0));
    n.fills.clear();
    doc.insert(n, parent, doc.index_in_parent(top).map(|i| i + 1))?;
    for i in ids {
        doc.reparent(*i, set, None)?;
    }
    Ok(set)
}

const INSIDE_INSTANCE: &str = "components inside an instance can't become variants";

/// Why `ids` (distinct) can't become the variants of a new component set, or `None` when they
/// can: anything but main components, components that are variants already (combining them would
/// put a set in a set), library copies, components inside an instance, and a component inside
/// another one of `ids`.
fn combine_refusal(doc: &Document, ids: &[NodeId]) -> Option<&'static str> {
    if !ids.iter().all(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. }))) {
        Some("select components only")
    } else if ids.iter().any(|i| variant_set(doc, *i).is_some()) {
        Some("components that are variants already can't be combined again")
    } else if ids.iter().any(|i| doc.is_remote(*i)) {
        Some(crate::library::READ_ONLY)
    } else if ids.iter().any(|i| inside_instance(doc, *i)) {
        Some(INSIDE_INSTANCE)
    } else if roots(doc, ids).len() != ids.len() {
        Some("a component can't be combined with one inside it")
    } else {
        None
    }
}

/// Whether "Combine as variants" is on offer for `ids` (menus and the Design panel): two or more
/// main components that [`combine_refusal`] lets through.
pub fn can_combine_variants(doc: &Document, ids: &[NodeId]) -> bool {
    ids.len() >= 2 && combine_refusal(doc, ids).is_none() && ids.iter().collect::<BTreeSet<_>>().len() == ids.len()
}

/// Whether "Add variant" is on offer for `id`: a component set with variants, a variant, or a main
/// component not in a set yet (it becomes one); never a library copy or inside an instance.
pub fn can_add_variant(doc: &Document, id: NodeId) -> bool {
    let Some(n) = doc.node(id) else { return false };
    let local = !doc.is_remote(id) && !inside_instance(doc, id);
    match n.kind {
        NodeKind::ComponentSet { .. } => {
            local && n.children.iter().any(|c| doc.node(*c).is_some_and(|c| matches!(c.kind, NodeKind::Component { .. })))
        }
        NodeKind::Component { .. } => local,
        _ => false,
    }
}

/// Wrap `ids` (z-sorted main components) in a new component set, naming them "Property 1=Default",
/// "Property 1=VariantN" unless already in prop=value form. Returns the set.
fn make_variant_set(cmd: &str, doc: &mut Document, ids: &[NodeId]) -> Result<NodeId> {
    let set = wrap_in_set(cmd, doc, ids)?;
    for (k, i) in ids.iter().enumerate() {
        let node = doc.get_mut(*i)?;
        if !node.name.contains('=') {
            node.name = format!("Property 1={}", if k == 0 { "Default".to_string() } else { format!("Variant{}", k + 1) });
        }
    }
    Ok(set)
}

fn combine(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.combineVariants";
    let mut seen = BTreeSet::new();
    let ids: Vec<NodeId> = req_targets(cmd, s, p)?.into_iter().filter(|i| seen.insert(*i)).collect();
    let doc = s.doc_mut()?;
    if let Some(why) = combine_refusal(doc, &ids) {
        return Err(EngineError::Disabled(cmd.into(), why.into()));
    }
    let ids = z_sorted(doc, &ids);
    let set = make_variant_set(cmd, doc, &ids)?;
    s.set_selection(vec![set]);
    Ok(json!({"id": set}))
}

/// Grow a component set so it wraps its variants with 20 px of padding (right and bottom).
fn fit_set(doc: &mut Document, set: NodeId) {
    let Some(kids) = doc.node(set).map(|n| n.children.clone()) else { return };
    let (mut w, mut h) = (0.0f64, 0.0f64);
    for c in kids {
        if let Some(n) = doc.node(c) {
            let b = figcraft_geom::transform_rect_bbox(n.transform, n.local_rect());
            w = w.max(b.x1 + 20.0);
            h = h.max(b.y1 + 20.0);
        }
    }
    if let Some(n) = doc.node_mut(set) {
        n.width = n.width.max(w);
        n.height = n.height.max(h);
    }
}

fn add_variant(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.addVariant";
    let t = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a component set or component"))?;
    let doc = s.doc_mut()?;
    let (set, base) = match doc.get(t)?.kind {
        NodeKind::ComponentSet { .. } => {
            let last = doc.get(t)?.children.last().copied().ok_or_else(|| bad(cmd, "the component set is empty"))?;
            (t, last)
        }
        NodeKind::Component { .. } => match variant_set(doc, t) {
            Some(set) => (set, t),
            // A main component on its own becomes a set of itself, then gains the new variant.
            None => (make_variant_set(cmd, doc, &[t])?, t),
        },
        _ => return Err(bad(cmd, "select a component set or component")),
    };
    let options = variant_options(doc, set);
    let mut pairs = variant_props(&doc.get(base)?.name);
    let order: Vec<String> = options.iter().map(|(k, _)| k.clone()).collect();
    let existing: BTreeSet<BTreeMap<String, String>> =
        doc.get(set)?.children.iter().filter_map(|c| doc.node(*c)).map(|n| variant_props(&n.name)).collect();
    match (string(p, "property"), string(p, "value")) {
        (Some(prop), Some(value)) => {
            let (prop, value) = (prop.trim().to_string(), value.trim().to_string());
            if !options.iter().any(|(k, _)| *k == prop) {
                return Err(bad(cmd, format!("unknown variant property `{prop}`")));
            }
            if value.is_empty() || value.contains([',', '=']) {
                return Err(bad(cmd, "`value` must be non-empty without ',' or '='"));
            }
            pairs.insert(prop, value);
            if existing.contains(&pairs) {
                return Err(bad(cmd, "that variant already exists"));
            }
        }
        (Some(_), None) => return Err(bad(cmd, "`value` is required with `property`")),
        _ => {
            let prop = order.first().cloned().unwrap_or_else(|| "Property 1".into());
            let mut k = existing.len() + 1;
            loop {
                pairs.insert(prop.clone(), format!("Variant{k}"));
                if !existing.contains(&pairs) || k > 10_000 {
                    break;
                }
                k += 1;
            }
        }
    }
    let name = variant_name(&pairs, &order);
    let bottom = doc
        .get(set)?
        .children
        .iter()
        .filter_map(|c| doc.node(*c))
        .map(|n| figcraft_geom::transform_rect_bbox(n.transform, n.local_rect()).y1)
        .fold(0.0f64, f64::max);
    let dup = doc.duplicate_into(base, set, None)?;
    let n = doc.get_mut(dup)?;
    n.name = name;
    let c = n.transform.as_coeffs();
    n.transform = figcraft_geom::Affine::new([c[0], c[1], c[2], c[3], c[4], bottom + 20.0]);
    fit_set(doc, set);
    s.set_selection(vec![dup]);
    Ok(json!({"id": dup}))
}

fn detach(s: &mut Session, p: &Value) -> Result<Value> {
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

fn reset_instance(s: &mut Session, p: &Value) -> Result<Value> {
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

fn reset_override(s: &mut Session, p: &Value) -> Result<Value> {
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
fn preferred_arg(cmd: &str, doc: &Document, p: &Value) -> Result<Vec<NodeId>> {
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

fn make_slot(s: &mut Session, p: &Value) -> Result<Value> {
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

fn remove_slot(s: &mut Session, p: &Value) -> Result<Value> {
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

fn slot_insert(s: &mut Session, p: &Value) -> Result<Value> {
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

fn reset_slot(s: &mut Session, p: &Value) -> Result<Value> {
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

fn push_changes(s: &mut Session, p: &Value) -> Result<Value> {
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

fn restore_main(s: &mut Session, p: &Value) -> Result<Value> {
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
    let mut nodes = Vec::new();
    let mut nested = Vec::new();
    for d in &sub {
        let Some(mut n) = doc.node(*d).cloned() else { continue };
        if *d == i {
            let mut defs = BTreeMap::new();
            for (k, v) in &props {
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
            n.kind = NodeKind::Component { description: String::new(), props: defs };
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

fn swap(s: &mut Session, p: &Value) -> Result<Value> {
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
    if doc.is_ancestor(c, i) {
        return Err("an instance can't show the component it is inside".into());
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

/// Whether instance `id` has a component property `name` that a `kind` variable can drive (a
/// boolean property takes a boolean variable, a text or variant property a string one).
pub(crate) fn check_prop_binding(doc: &Document, id: NodeId, name: &str, kind: VariableType) -> std::result::Result<(), String> {
    let Some(NodeKind::Instance { component, .. }) = doc.node(id).map(|n| &n.kind) else {
        return Err(format!("{id} is not an instance (component properties are bound on instances)"));
    };
    if variant_set(doc, *component).is_some_and(|set| variant_options(doc, set).iter().any(|(o, _)| o == name)) {
        return if kind == VariableType::String { Ok(()) } else { Err(format!("`{name}` is a variant property: bind a string variable")) };
    }
    match (prop_defs(doc, *component).get(name), kind) {
        (Some(PropDef::Boolean { .. }), VariableType::Boolean) | (Some(PropDef::Text { .. }), VariableType::String) => Ok(()),
        (Some(PropDef::Boolean { .. }), _) => Err(format!("`{name}` is a boolean property: bind a boolean variable")),
        (Some(PropDef::Text { .. }), _) => Err(format!("`{name}` is a text property: bind a string variable")),
        (Some(_), _) => Err(format!("`{name}`: only boolean and text properties can be bound to variables")),
        (None, _) => Err(format!("the instance has no property named `{name}`")),
    }
}

/// Write bound variable values into instance `id`'s boolean and text property values, and switch
/// it to the variant its bound variant properties name (only what changed; the instance then
/// follows like after `instance.setProperties`). A value that names no variant changes nothing.
pub(crate) fn apply_prop_bindings(doc: &mut Document, id: NodeId, sets: &[(String, VariableValue)]) {
    let Some(NodeKind::Instance { component, .. }) = doc.node(id).map(|n| &n.kind) else { return };
    let component = *component;
    if let Some(set) = variant_set(doc, component) {
        let options = variant_options(doc, set);
        let current = doc.node(component).map(|c| variant_props(&c.name)).unwrap_or_default();
        let mut wanted = current.clone();
        for (name, val) in sets {
            if let VariableValue::String(t) = val
                && options.iter().any(|(o, vals)| o == name && vals.contains(t))
            {
                wanted.insert(name.clone(), t.clone());
            }
        }
        if wanted != current
            && let Some(target) = find_variant(doc, set, &wanted)
            && let Ok(n) = doc.get_mut(id)
            && let NodeKind::Instance { component: c, .. } = &mut n.kind
        {
            *c = target;
        }
    }
    let Some(NodeKind::Instance { component, props }) = doc.node(id).map(|n| &n.kind) else { return };
    let Some(NodeKind::Component { props: defs, .. }) = doc.node(*component).map(|c| &c.kind) else { return };
    let mut next = props.clone();
    for (name, val) in sets {
        let v = match (defs.get(name), val) {
            (Some(PropDef::Boolean { .. }), VariableValue::Boolean(b)) => json!(b),
            (Some(PropDef::Text { .. }), VariableValue::String(t)) => json!(t),
            _ => continue,
        };
        next.insert(name.clone(), v);
    }
    if next != *props
        && let Ok(n) = doc.get_mut(id)
        && let NodeKind::Instance { props, .. } = &mut n.kind
    {
        *props = next;
    }
}

/// Coerce a property value to the definition's type.
fn coerce(cmd: &str, doc: &Document, name: &str, def: &PropDef, v: &Value) -> Result<Value> {
    match def {
        PropDef::Boolean { .. } => match v {
            Value::Bool(b) => Ok(json!(b)),
            Value::String(t) if t == "true" || t == "false" => Ok(json!(t == "true")),
            _ => Err(bad(cmd, format!("`{name}` is a boolean property"))),
        },
        PropDef::Text { .. } => match v {
            Value::String(t) => Ok(json!(t)),
            Value::Number(n) => Ok(json!(n.to_string())),
            _ => Err(bad(cmd, format!("`{name}` is a text property"))),
        },
        PropDef::InstanceSwap { .. } => match id_of(v).and_then(|c| main_of(doc, c)) {
            Some(c) => Ok(json!(c.0)),
            None => Err(bad(cmd, format!("`{name}` takes a component id"))),
        },
        PropDef::Variant { options, .. } => {
            let t = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
            if options.is_empty() || options.contains(&t) { Ok(json!(t)) } else { Err(bad(cmd, format!("`{name}` must be one of {options:?}"))) }
        }
        PropDef::Slot { .. } => Err(bad(cmd, format!("`{name}` is a slot: fill it with instance.slotInsert or by adding layers to it"))),
    }
}

fn set_props(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "instance.setProperties";
    let ids = req_targets(cmd, s, p)?;
    let props = get(p, "properties").and_then(Value::as_object).cloned().ok_or_else(|| bad(cmd, "`properties` object required"))?;
    let doc = s.doc_mut()?;
    for i in ids {
        let Some(NodeKind::Instance { component, .. }) = doc.node(i).map(|n| n.kind.clone()) else {
            return Err(bad(cmd, format!("{i} is not an instance")));
        };
        // Variant properties: switch to the matching sibling variant.
        let set = variant_set(doc, component);
        let options = set.map(|s| variant_options(doc, s)).unwrap_or_default();
        let mut wanted = doc.node(component).map(|c| variant_props(&c.name)).unwrap_or_default();
        let mut target = component;
        let mut rest = Vec::new();
        for (k, v) in &props {
            if let Some((_, vals)) = options.iter().find(|(o, _)| o == k) {
                let val = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                if !vals.contains(&val) {
                    return Err(bad(cmd, format!("`{k}` must be one of {vals:?}")));
                }
                wanted.insert(k.clone(), val);
            } else {
                rest.push((k.clone(), v.clone()));
            }
        }
        if let Some(set) = set
            && wanted != doc.node(component).map(|c| variant_props(&c.name)).unwrap_or_default()
        {
            target = find_variant(doc, set, &wanted).unwrap_or(component);
        }
        let defs = match doc.node(target).map(|n| &n.kind) {
            Some(NodeKind::Component { props, .. }) => props.clone(),
            _ => BTreeMap::new(),
        };
        let mut values = Vec::new();
        for (k, v) in rest {
            let def = defs.get(&k).ok_or_else(|| {
                let known: Vec<&str> = defs.keys().map(String::as_str).chain(options.iter().map(|(o, _)| o.as_str())).collect();
                bad(cmd, format!("unknown property `{k}` (known: {})", known.join(", ")))
            })?;
            values.push((k.clone(), coerce(cmd, doc, &k, def, &v)?));
        }
        if let NodeKind::Instance { component: c, props: ip } = &mut doc.get_mut(i)?.kind {
            *c = target;
            for (k, v) in values {
                ip.insert(k, v);
            }
            for (k, d) in defs.iter().filter(|(_, d)| takes_value(d)) {
                ip.entry(k.clone()).or_insert_with(|| prop_default_value(d));
            }
        }
    }
    ok()
}

/// Where component properties live: a set (its variants share them) or a lone component.
enum Owner {
    Set(NodeId),
    Comp(NodeId),
}

impl Owner {
    fn resolve(doc: &Document, id: NodeId) -> Option<Owner> {
        let n = doc.node(id)?;
        match n.kind {
            NodeKind::ComponentSet { .. } => Some(Owner::Set(id)),
            _ => {
                let c = owning_component(doc, id)?;
                Some(match variant_set(doc, c) {
                    Some(s) => Owner::Set(s),
                    None => Owner::Comp(c),
                })
            }
        }
    }

    fn components(&self, doc: &Document) -> Vec<NodeId> {
        match self {
            Owner::Comp(c) => vec![*c],
            Owner::Set(s) => doc
                .node(*s)
                .map(|n| n.children.iter().copied().filter(|c| doc.node(*c).is_some_and(|x| matches!(x.kind, NodeKind::Component { .. }))).collect())
                .unwrap_or_default(),
        }
    }

    fn id(&self) -> NodeId {
        match self {
            Owner::Set(s) | Owner::Comp(s) => *s,
        }
    }
}

fn prop_defs(doc: &Document, c: NodeId) -> BTreeMap<String, PropDef> {
    match doc.node(c).map(|n| &n.kind) {
        Some(NodeKind::Component { props, .. }) => props.clone(),
        _ => BTreeMap::new(),
    }
}

fn set_prop_defs(doc: &mut Document, c: NodeId, f: impl FnOnce(&mut BTreeMap<String, PropDef>)) -> Result<()> {
    if let NodeKind::Component { props, .. } = &mut doc.get_mut(c)?.kind {
        f(props);
    }
    Ok(())
}

fn valid_prop_name(cmd: &str, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.contains([',', '=']) || name.chars().count() > 200 {
        return Err(bad(cmd, "property names must be non-empty, without ',' or '='"));
    }
    Ok(name.to_string())
}

fn add_prop(s: &mut Session, p: &Value) -> Result<Value> {
    add_prop_for("component.addProperty", s, p)
}

/// Add a component property for command `cmd`, which errors name (`component.makeSlot` adds its
/// slot property here).
fn add_prop_for(cmd: &str, s: &mut Session, p: &Value) -> Result<Value> {
    let target = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a component"))?;
    let name = valid_prop_name(cmd, &req_string(cmd, p, "name")?)?;
    let t = req_string(cmd, p, "type")?;
    let doc = s.doc_mut()?;
    if inside_instance(doc, target) {
        return Err(bad(cmd, "select a component, not a layer of an instance"));
    }
    let owner = Owner::resolve(doc, target).ok_or_else(|| bad(cmd, "select a component, a component set or a layer inside a component"))?;
    // A layer selected inside the component gets bound to the new property.
    let layer = id(p, "layer")
        .or_else(|| (target != owner.id() && !matches!(doc.node(target).map(|n| &n.kind), Some(NodeKind::Component { .. }))).then_some(target));
    let comps = owner.components(doc);
    let variants = match &owner {
        Owner::Set(s) => variant_options(doc, *s),
        Owner::Comp(_) => Vec::new(),
    };
    if variants.iter().any(|(k, _)| *k == name) || comps.iter().any(|c| prop_defs(doc, *c).contains_key(&name)) {
        return Err(bad(cmd, format!("a property named `{name}` already exists")));
    }
    let layer_node = layer.and_then(|l| doc.node(l)).cloned();
    let def = match t.to_ascii_lowercase().as_str() {
        "boolean" | "bool" => PropDef::Boolean { default: boolean(p, "default").unwrap_or(true) },
        "text" => PropDef::Text {
            default: string(p, "default")
                .or_else(|| layer_node.as_ref().and_then(|n| n.kind.text()).map(|t| t.characters.clone()))
                .unwrap_or_else(|| name.clone()),
        },
        "instanceswap" | "instance_swap" | "instance" => {
            let d = id(p, "default")
                .or_else(|| match layer_node.as_ref().map(|n| &n.kind) {
                    Some(NodeKind::Instance { component, .. }) => Some(*component),
                    _ => None,
                })
                .and_then(|c| main_of(doc, c))
                .ok_or_else(|| bad(cmd, "default: a component id"))?;
            PropDef::InstanceSwap { default: d, preferred: preferred_arg(cmd, doc, p)? }
        }
        "variant" => {
            let dv = string(p, "default")
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty() && !d.contains([',', '=']))
                .unwrap_or_else(|| "Default".into());
            let set = match owner {
                Owner::Set(s) => s,
                Owner::Comp(c) => wrap_in_set(cmd, doc, &[c])?,
            };
            for c in Owner::Set(set).components(doc) {
                let cn = doc.get_mut(c)?;
                cn.name = if cn.name.contains('=') { format!("{}, {name}={dv}", cn.name) } else { format!("{name}={dv}") };
            }
            s.set_selection(vec![set]);
            return Ok(json!({"id": set}));
        }
        "slot" => {
            if !layer_node.as_ref().is_some_and(|n| matches!(n.kind, NodeKind::Frame)) {
                return Err(bad(cmd, "a slot is a frame inside the component: select one (or pass `layer`)"));
            }
            PropDef::Slot { preferred: preferred_arg(cmd, doc, p)? }
        }
        _ => return Err(bad(cmd, "type: boolean|text|instanceSwap|variant|slot")),
    };
    for c in &comps {
        set_prop_defs(doc, *c, |m| {
            m.insert(name.clone(), def.clone());
        })?;
        // Existing instances get the default.
        if !takes_value(&def) {
            continue;
        }
        for inst in instances_of(doc, *c) {
            if let NodeKind::Instance { props, .. } = &mut doc.get_mut(inst)?.kind {
                props.entry(name.clone()).or_insert_with(|| prop_default_value(&def));
            }
        }
    }
    if let Some(l) = layer {
        bind(cmd, doc, l, Some(&name), None)?;
    }
    Ok(json!({"id": owner.id(), "name": name}))
}

/// Bind (or with `property: None` unbind) a layer's field to a property of its component.
fn bind(cmd: &str, doc: &mut Document, layer: NodeId, property: Option<&str>, field: Option<&str>) -> Result<()> {
    let n = doc.get(layer)?.clone();
    if inside_instance(doc, layer) {
        return Err(bad(cmd, "layers inside an instance can't be bound (bind the nested instance itself, or edit its main)"));
    }
    let comp = owning_component(doc, layer).ok_or_else(|| bad(cmd, "the layer isn't inside a component"))?;
    let Some(property) = property.filter(|p| !p.is_empty()) else {
        let n = doc.get_mut(layer)?;
        match field {
            Some(f) => {
                n.prop_refs.remove(f);
            }
            None => n.prop_refs.clear(),
        }
        return Ok(());
    };
    let defs = prop_defs(doc, comp);
    let def = defs.get(property).ok_or_else(|| bad(cmd, format!("the component has no property `{property}`")))?;
    let expected = match def {
        PropDef::Boolean { .. } => "visible",
        PropDef::Text { .. } => "characters",
        PropDef::InstanceSwap { .. } => "mainComponent",
        PropDef::Variant { .. } => return Err(bad(cmd, "variant properties pick a variant; they can't be bound to a layer")),
        PropDef::Slot { .. } => SLOT,
    };
    let field = field.unwrap_or(expected);
    if field != expected {
        return Err(bad(cmd, format!("`{property}` can only control `{expected}`")));
    }
    match field {
        "characters" if n.kind.text().is_none() => return Err(bad(cmd, "text properties bind to text layers")),
        "mainComponent" if !matches!(n.kind, NodeKind::Instance { .. }) => return Err(bad(cmd, "instance swap properties bind to instances")),
        "visible" if layer == comp => return Err(bad(cmd, "the component itself can't be hidden by its own property")),
        SLOT if !matches!(n.kind, NodeKind::Frame) => return Err(bad(cmd, "slot properties bind to frames inside the component")),
        SLOT if doc
            .subtree(comp)
            .into_iter()
            .any(|d| d != layer && doc.node(d).is_some_and(|x| x.prop_refs.get(SLOT).map(String::as_str) == Some(property))) =>
        {
            return Err(bad(cmd, format!("another frame of the component is already the slot `{property}`")));
        }
        _ => {}
    }
    doc.get_mut(layer)?.prop_refs.insert(field.to_string(), property.to_string());
    Ok(())
}

fn bind_prop(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.bindProperty";
    let layer = id(p, "layer").or_else(|| targets(s, p).first().copied()).ok_or_else(|| bad(cmd, "`layer` required"))?;
    let property = string(p, "property");
    let field = string(p, "field");
    if let Some(f) = &field
        && !matches!(f.as_str(), "visible" | "characters" | "mainComponent" | SLOT)
    {
        return Err(bad(cmd, "field: visible|characters|mainComponent|slot"));
    }
    let doc = s.doc_mut()?;
    bind(cmd, doc, layer, property.as_deref(), field.as_deref())?;
    ok()
}

/// Rename `old` → `new` in the bindings of a component's layers and in its instances' values.
fn rename_prop_refs(doc: &mut Document, comp: NodeId, old: &str, new: Option<&str>) -> Result<()> {
    for d in doc.subtree(comp) {
        let refs = doc.node(d).map(|n| n.prop_refs.clone()).unwrap_or_default();
        if refs.values().any(|v| v == old)
            && let Some(n) = doc.node_mut(d)
        {
            match new {
                Some(nw) => n.prop_refs.values_mut().filter(|v| *v == old).for_each(|v| *v = nw.to_string()),
                None => n.prop_refs.retain(|_, v| v != old),
            }
        }
    }
    // Instances' values and the variables bound to them follow (a deleted property drops both).
    let old_key = format!("{PROP_BINDING}{old}");
    for inst in instances_of(doc, comp) {
        let n = doc.get_mut(inst)?;
        if let NodeKind::Instance { props, .. } = &mut n.kind
            && let Some(v) = props.remove(old)
            && let Some(nw) = new
        {
            props.insert(nw.to_string(), v);
        }
        if let Some(var) = n.bound.remove(&old_key)
            && let Some(nw) = new
        {
            n.bound.insert(format!("{PROP_BINDING}{nw}"), var);
        }
    }
    Ok(())
}

fn edit_prop(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.editProperty";
    let target = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a component"))?;
    let name = req_string(cmd, p, "name")?;
    let new_name = string(p, "newName").map(|n| valid_prop_name(cmd, &n)).transpose()?.filter(|n| *n != name);
    let doc = s.doc_mut()?;
    let owner = Owner::resolve(doc, target).ok_or_else(|| bad(cmd, "select a component or component set"))?;
    let comps = owner.components(doc);
    if let Some(nn) = &new_name {
        let taken = comps.iter().any(|c| prop_defs(doc, *c).contains_key(nn))
            || matches!(owner, Owner::Set(s) if variant_options(doc, s).iter().any(|(k, _)| k == nn));
        if taken {
            return Err(bad(cmd, format!("a property named `{nn}` already exists")));
        }
    }
    // Variant property of a set.
    if let Owner::Set(set) = owner
        && let Some((_, values)) = variant_options(doc, set).into_iter().find(|(k, _)| *k == name)
    {
        let order: Vec<String> = variant_options(doc, set).into_iter().map(|(k, _)| k).collect();
        let rename_value = match (string(p, "value"), string(p, "newValue")) {
            (Some(v), Some(nv)) => {
                let nv = nv.trim().to_string();
                if !values.contains(&v) {
                    return Err(bad(cmd, format!("`{name}` has no value `{v}`")));
                }
                if nv.is_empty() || nv.contains([',', '=']) {
                    return Err(bad(cmd, "`newValue` must be non-empty, without ',' or '='"));
                }
                Some((v, nv))
            }
            _ => None,
        };
        let mut order: Vec<String> = order.into_iter().map(|k| if k == name { new_name.clone().unwrap_or(k) } else { k }).collect();
        // Reordered: every variant's name lists the properties in the new order.
        if let Some(to) = get(p, "index").filter(|v| !v.is_null()) {
            let to = to.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(cmd, "`index`: a property place (0 = first)"))?;
            let key = new_name.clone().unwrap_or(name.clone());
            order.retain(|k| *k != key);
            order.insert(to.min(order.len()), key);
        }
        for c in &comps {
            let mut pairs = variant_props(&doc.get(*c)?.name);
            if let Some((v, nv)) = &rename_value
                && pairs.get(&name) == Some(v)
            {
                pairs.insert(name.clone(), nv.clone());
            }
            if let Some(nn) = &new_name
                && let Some(v) = pairs.remove(&name)
            {
                pairs.insert(nn.clone(), v);
            }
            doc.get_mut(*c)?.name = variant_name(&pairs, &order);
        }
        // The default variant is the top-left one: the variant with that value (and the default's
        // other values, when there is one) trades places with it.
        if let Some(dv) = string(p, "default") {
            let key = new_name.clone().unwrap_or(name.clone());
            let current = default_variant(doc, set);
            let mut wanted = current.and_then(|d| doc.node(d)).map(|n| variant_props(&n.name)).unwrap_or_default();
            wanted.insert(key.clone(), dv.clone());
            let props_of = |c: &NodeId| doc.node(*c).map(|n| variant_props(&n.name)).unwrap_or_default();
            let pick =
                comps.iter().copied().find(|c| props_of(c) == wanted).or_else(|| comps.iter().copied().find(|c| props_of(c).get(&key) == Some(&dv)));
            let (Some(f), Some(d)) = (pick, current) else { return Err(bad(cmd, format!("no variant has {key}={dv}"))) };
            if f != d {
                let place = |doc: &Document, c: NodeId| doc.node(c).map(|n| n.transform.as_coeffs()).map(|[.., x, y]| (x, y)).unwrap_or_default();
                let (fp, dp) = (place(doc, f), place(doc, d));
                for (c, (x, y)) in [(f, dp), (d, fp)] {
                    let n = doc.get_mut(c)?;
                    let [a, b, cc, dd, ..] = n.transform.as_coeffs();
                    n.transform = figcraft_geom::Affine::new([a, b, cc, dd, x, y]);
                }
                fit_set(doc, set);
            }
        }
        return ok();
    }
    let def = comps.iter().find_map(|c| prop_defs(doc, *c).get(&name).cloned()).ok_or_else(|| bad(cmd, format!("no property named `{name}`")))?;
    let preferred = if get(p, "preferred").is_some() {
        if !matches!(def, PropDef::Slot { .. } | PropDef::InstanceSwap { .. }) {
            return Err(bad(cmd, "`preferred` is for instance swap and slot properties"));
        }
        Some(preferred_arg(cmd, doc, p)?)
    } else {
        None
    };
    let mut new_def = match get(p, "default") {
        Some(v) => match coerce(cmd, doc, &name, &def, v)? {
            Value::Bool(b) => PropDef::Boolean { default: b },
            Value::String(t) if matches!(def, PropDef::Text { .. }) => PropDef::Text { default: t },
            other => match (&def, id_of(&other)) {
                (PropDef::InstanceSwap { preferred, .. }, Some(c)) => PropDef::InstanceSwap { default: c, preferred: preferred.clone() },
                _ => def.clone(),
            },
        },
        None => def.clone(),
    };
    if let (Some(list), PropDef::Slot { preferred } | PropDef::InstanceSwap { preferred, .. }) = (preferred, &mut new_def) {
        *preferred = list;
    }
    for c in &comps {
        let had = prop_defs(doc, *c).contains_key(&name);
        if !had {
            continue;
        }
        let key = new_name.clone().unwrap_or(name.clone());
        let nd = new_def.clone();
        let old = name.clone();
        set_prop_defs(doc, *c, move |m| {
            m.remove(&old);
            m.insert(key, nd);
        })?;
        if let Some(nn) = &new_name {
            rename_prop_refs(doc, *c, &name, Some(nn))?;
        }
    }
    ok()
}

fn delete_prop(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.deleteProperty";
    let target = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a component"))?;
    let name = req_string(cmd, p, "name")?;
    let doc = s.doc_mut()?;
    let owner = Owner::resolve(doc, target).ok_or_else(|| bad(cmd, "select a component or component set"))?;
    let comps = owner.components(doc);
    if let Owner::Set(set) = owner
        && variant_options(doc, set).iter().any(|(k, _)| *k == name)
    {
        let order: Vec<String> = variant_options(doc, set).into_iter().map(|(k, _)| k).collect();
        for c in &comps {
            let mut pairs = variant_props(&doc.get(*c)?.name);
            pairs.remove(&name);
            let n = doc.get_mut(*c)?;
            n.name = if pairs.is_empty() { "Default".into() } else { variant_name(&pairs, &order) };
        }
        return ok();
    }
    let mut found = false;
    for c in &comps {
        if prop_defs(doc, *c).contains_key(&name) {
            found = true;
            set_prop_defs(doc, *c, |m| {
                m.remove(&name);
            })?;
            rename_prop_refs(doc, *c, &name, None)?;
        }
    }
    if !found {
        return Err(bad(cmd, format!("no property named `{name}`")));
    }
    ok()
}

fn def_json(name: &str, d: &PropDef) -> Value {
    match d {
        PropDef::Boolean { default } => json!({"name": name, "type": "boolean", "default": default}),
        PropDef::Text { default } => json!({"name": name, "type": "text", "default": default}),
        PropDef::InstanceSwap { default, preferred } => json!({"name": name, "type": "instanceSwap", "default": default, "preferred": preferred}),
        PropDef::Variant { options, default } => json!({"name": name, "type": "variant", "default": default, "options": options}),
        PropDef::Slot { preferred } => json!({"name": name, "type": "slot", "preferred": preferred}),
    }
}

/// Properties of a component, set or instance as JSON (`component.properties`).
pub(crate) fn describe_props(doc: &Document, id: NodeId) -> Value {
    let Some(n) = doc.node(id) else { return Value::Null };
    let (owner_comp, values) = match &n.kind {
        NodeKind::Instance { component, props } => (Some(*component), Some(props.clone())),
        _ => (main_of(doc, id).or_else(|| owning_component(doc, id)), None),
    };
    let Some(comp) = owner_comp else { return json!({"properties": []}) };
    let mut list: Vec<Value> = Vec::new();
    if let Some(set) = variant_set(doc, comp) {
        let current = doc.node(comp).map(|c| variant_props(&c.name)).unwrap_or_default();
        let defaults = default_variant(doc, set).and_then(|d| doc.node(d)).map(|n| variant_props(&n.name)).unwrap_or_default();
        for (k, vals) in variant_options(doc, set) {
            let mut v = json!({"name": k, "type": "variant", "options": vals, "default": defaults.get(&k).or(vals.first())});
            if values.is_some() {
                v["value"] = json!(current.get(&k));
            }
            list.push(v);
        }
    }
    // Slots: the frames (on a main) or this instance's copy, with whether it holds its own content.
    let slots = slots_of(doc, if values.is_some() { id } else { variant_set(doc, comp).unwrap_or(comp) });
    for (k, d) in prop_defs(doc, comp) {
        let mut v = def_json(&k, &d);
        if !takes_value(&d) {
            let layers: Vec<NodeId> = slots.iter().filter(|(_, n)| *n == k).map(|(l, _)| *l).collect();
            if values.is_some() {
                v["value"] = layers.first().map_or(Value::Null, |l| {
                    let count = doc.node(*l).map_or(0, |n| n.children.len());
                    json!({"layer": l, "edited": slot_edited(doc, *l), "layers": count})
                });
            } else {
                v["layers"] = json!(layers);
            }
        } else if let Some(vals) = &values {
            v["value"] = vals.get(&k).cloned().unwrap_or_else(|| prop_default_value(&d));
        }
        list.push(v);
    }
    let set = variant_set(doc, comp);
    let conflicts = set.map(|s| variant_conflicts(doc, s)).unwrap_or_default();
    json!({"component": comp, "set": set, "properties": list, "conflicts": conflicts})
}
