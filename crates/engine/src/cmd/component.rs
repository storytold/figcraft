//! Component commands: components, variants, component properties and instances.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::{Document, Node, NodeId, NodeKind, PropDef, VariableType, VariableValue};
use serde_json::{Value, json};

use super::edit::{roots, z_sorted};
use super::{CommandSpec, ok};
use crate::components::{
    OVERRIDE_GROUPS, PROP_BINDING, SLOT, copy_group, create_instance, create_instance_at, default_props, default_variant, exposed_copies,
    find_variant, inside_instance, instances_of, is_slot, keep_text_overrides, owning_component, owning_instance, prop_default_value,
    rebase_instance, root_instance, set_boolean_default, slot_edited, slot_name, slots_of, takes_value, unlink_layers, variant_conflicts,
    variant_name, variant_options, variant_props, variant_set,
};
use crate::params::{boolean, get, id, id_of, num, req_id, req_num, req_string, req_targets, string, targets};
use crate::{EngineError, Result, Session, bad, spec};

mod instance;
mod props;
mod variants;

use instance::{detach, make_slot, push_changes, remove_slot, reset_instance, reset_override, reset_slot, restore_main, slot_insert, swap};
pub(crate) use instance::{swap_at, swap_instance};
use props::{add_prop, bind_prop, delete_prop, edit_prop, set_props};
pub(crate) use props::{apply_prop_bindings, check_prop_binding, describe_props};
use variants::{add_variant, combine};
pub use variants::{can_add_variant, can_combine_variants};

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
            crate::library::count_inserted(doc, &[i]);
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
            "{id?, group? (fills|strokes|effects|opacity|visible|name|corners|characters|textStyle|padding|gap|alignment|grids|exports|interactions|size|position|modes|component|props|slot; all when omitted)}",
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
            "{id? (component, set, or a layer inside a component: it gets bound), name, type: boolean|text|instanceSwap|variant|slot, default? (boolean: the bound layer's visibility in the main; without it, the layer's current visibility), layer? (node bound to it; a frame for slots), preferred? (instance swap and slot properties: component or set ids offered first)}",
            true,
            add_prop
        ),
        spec!(
            "component.editProperty",
            "Edit component property",
            "{id? (component or set), name, newName?, default? (variant properties: that variant takes the top-left place; boolean: the main's bound layers show or hide to match), value? + newValue? (rename a variant value), index? (variant properties: move to this place; the variants' names follow), preferred? (instance swap and slot properties: component or set ids offered first)}",
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
    // Never inside its own main component, or one it contains (directly or through others).
    if crate::components::would_contain_itself(doc, c, slot.parent) {
        slot = crate::drop::DropSlot { parent: page, index: None, line: None };
    }
    let (w, h) = doc.node(c).map(|n| (n.width, n.height)).unwrap_or((0.0, 0.0));
    let local = doc.world_transform(slot.parent).inverse() * figcraft_geom::Point::new(pt.x - w / 2.0, pt.y - h / 2.0);
    let i = crate::components::create_instance_at(doc, c, slot.parent, local.x, local.y, slot.index)?;
    crate::library::count_inserted(doc, &[i]);
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
    n.kind = NodeKind::Component { description: String::new(), props: Default::default(), prop_order: Vec::new() };
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
