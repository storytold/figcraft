//! Component, component-property and instance sections of the Design panel.
//!
//! - An instance: its main component (with a swap menu), variant dropdowns, property values,
//!   the selected layer's overrides, and Reset / Detach / Go to main / Push changes.
//! - A component or component set: its properties (add, rename, default, delete), Add variant
//!   (a lone main component becomes a set) and, for a set, the variant property matrix.
//! - Several main components: Combine as variants.
//! - A layer inside a component: bind its visibility / text / instance to a property, or make a
//!   frame a slot.
//! - A slot inside an instance (and an instance's slot properties): fill it with components
//!   (preferred ones first), select it, reset it to the main's default content.

use egui::{Id, Sense, Ui, vec2};
use figcraft_doc::{Document, Node, NodeId, NodeKind, PropDef, VariableType};
use figcraft_engine::cmd::component::{can_add_variant, can_combine_variants};
use figcraft_engine::components::{
    PROP_BINDING, default_variant, inside_instance, owning_component, owning_instance, prop_list, slot_def, slot_edited, slots_of, variant_conflicts,
    variant_options, variant_props, variant_set,
};
use serde_json::{Value, json};

use crate::FigApp;
use crate::i18n::{tr, tr_noop, trn};
use crate::icons::{self, Icon};
use crate::list_drag::ListDrag;
use crate::right::{menu_item, menu_popup, row, row_rect};
use crate::theme::{COMPONENT, Tokens, semibold};
use crate::widgets;

/// Entry point (called from the Design panel header for every selection).
pub fn instance_section(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let [id] = sel else {
        combine_ui(app, ui, doc, sel);
        return;
    };
    let Some(n) = doc.node(*id) else { return };
    match &n.kind {
        NodeKind::Instance { .. } => instance_ui(app, ui, doc, n),
        NodeKind::ComponentSet { description } => {
            properties_ui(app, ui, doc, n.id);
            description_ui(app, ui, n.id, description);
        }
        NodeKind::Component { description, .. } => {
            properties_ui(app, ui, doc, variant_set(doc, n.id).unwrap_or(n.id));
            description_ui(app, ui, n.id, description);
        }
        _ => {}
    }
    if owning_instance(doc, n.id).is_some()
        && let Some((name, preferred)) = slot_def(doc, n.id)
    {
        slot_section(app, ui, doc, n.id, name, preferred);
    }
    if !matches!(n.kind, NodeKind::Instance { .. }) && n.source.is_some() && owning_instance(doc, n.id).is_some() {
        overrides_ui(app, ui, n);
    }
    if !matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. })
        && !inside_instance(doc, n.id)
        && owning_component(doc, n.id).is_some()
    {
        bind_ui(app, ui, doc, n);
    }
}

/// Two or more main components selected: a button combines them into a component set.
fn combine_ui(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    if !can_combine_variants(doc, sel) {
        return;
    }
    row(ui, 32.0, |ui| {
        let label = crate::i18n::command("component.combineVariants", "Combine as variants");
        if widgets::dropdown(ui, &label, 160.0, Some(Icon::ComponentSet)).clicked() {
            app.run("component.combineVariants", json!({"ids": sel}));
        }
    });
}

/// A component's description: a multi-line field, saved when it loses focus.
fn description_ui(app: &mut FigApp, ui: &mut Ui, id: NodeId, description: &str) {
    widgets::separator(ui);
    widgets::sublabels(ui, tr("Description"), None);
    if let Some(new) = widgets::multiline_field(ui, Id::unique(("comp_desc", id)), description, tr("Describe how and when to use this component"), 3)
    {
        app.run("node.set", json!({"id": id, "props": {"description": new.chars().take(10_000).collect::<String>()}}));
    }
    ui.add_space(8.0);
}

/// Local components (and component sets, not their variants), for swap menus: (id, label), page
/// by page. Listed once per document revision (a file with thousands of layers isn't walked every
/// frame).
pub(crate) fn local_components(app: &FigApp, ui: &Ui, doc: &Document) -> std::sync::Arc<Vec<(NodeId, String)>> {
    type Cached = ((u64, u64), std::sync::Arc<Vec<(NodeId, String)>>);
    let key = app.session.ds().map(|d| (d.uid, d.revision)).unwrap_or_default();
    let id = Id::unique("component_panel_local_components");
    if let Some((k, list)) = ui.data(|d| d.get_temp::<Cached>(id))
        && k == key
    {
        return list;
    }
    let list = std::sync::Arc::new(list_components(doc));
    ui.data_mut(|d| d.insert_temp::<Cached>(id, (key, list.clone())));
    list
}

fn list_components(doc: &Document) -> Vec<(NodeId, String)> {
    let mut out: Vec<(NodeId, String)> = Vec::new();
    for page in &doc.pages {
        for d in doc.subtree(*page) {
            let Some(n) = doc.node(d) else { continue };
            let is = match n.kind {
                NodeKind::Component { .. } => variant_set(doc, d).is_none(),
                NodeKind::ComponentSet { .. } => true,
                _ => false,
            };
            if is && !inside_instance(doc, d) {
                out.push((d, n.name.clone()));
            }
        }
    }
    out
}

/// A section header row with a title and a "+" button; returns the button response.
fn header_with_plus(ui: &mut Ui, title: &str, tip: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let mut plus = None;
    row(ui, 36.0, |ui| {
        ui.label(egui::RichText::new(title).font(semibold(11.0)).color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            plus = Some(icons::button(ui, Icon::Plus, 24.0, false, tip));
        });
    });
    match plus {
        Some(p) => p,
        None => ui.allocate_response(vec2(0.0, 0.0), Sense::hover()),
    }
}

fn type_icon(d: &PropDef) -> Icon {
    match d {
        PropDef::Boolean { .. } => Icon::Eye,
        PropDef::Text { .. } => Icon::Text,
        PropDef::InstanceSwap { .. } => Icon::Instance,
        PropDef::Variant { .. } => Icon::ComponentSet,
        PropDef::Slot { .. } => Icon::Slot,
    }
}

/// A selected slot inside an instance: its fill menu.
fn slot_section(app: &mut FigApp, ui: &mut Ui, doc: &Document, slot: NodeId, name: &str, preferred: &[NodeId]) {
    let t = Tokens::get(ui.ctx());
    let comps = local_components(app, ui, doc);
    widgets::separator(ui);
    row(ui, 36.0, |ui| {
        ui.label(egui::RichText::new(tr("Slot")).font(semibold(11.0)).color(t.text));
    });
    row(ui, 32.0, |ui| {
        prop_label(ui, name, Icon::Slot);
        let w = ui.available_width().max(60.0);
        slot_fill(app, ui, doc, slot, preferred, &comps, w);
    });
}

/// The fill menu of slot `slot` inside an instance: its preferred components first, then every
/// local component; select the slot; reset it to the main's default content.
fn slot_fill(app: &mut FigApp, ui: &mut Ui, doc: &Document, slot: NodeId, preferred: &[NodeId], comps: &[(NodeId, String)], w: f32) {
    let t = Tokens::get(ui.ctx());
    let edited = slot_edited(doc, slot);
    let label = match (edited, doc.node(slot).map_or(0, |n| n.children.len())) {
        (false, 0) => tr("Empty").to_string(),
        (false, _) => tr("Default content").to_string(),
        (true, k) => trn("{n} layers", k, &[]),
    };
    let resp = widgets::dropdown(ui, &label, w, Some(Icon::Slot)).on_hover_text(tr("Add content to this slot"));
    menu_popup(&resp).width(w.max(200.0)).show(|ui| {
        let insert = |app: &mut FigApp, c: NodeId| app.run("instance.slotInsert", json!({"id": slot, "component": c.0}));
        let heading = |ui: &mut Ui, text: &str| ui.label(egui::RichText::new(text).font(semibold(11.0)).color(t.text_secondary));
        if !preferred.is_empty() {
            heading(ui, tr("Preferred"));
            for c in preferred.iter().filter(|c| doc.contains(**c)) {
                let name = doc.node(*c).map(|n| n.name.clone()).unwrap_or_default();
                if menu_item(ui, &name, "") {
                    insert(app, *c);
                    ui.close();
                }
            }
        }
        heading(ui, tr("All components"));
        egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
            // Never a component the slot is part of.
            for (c, name) in comps.iter().filter(|(c, _)| !doc.is_ancestor(*c, slot)) {
                if menu_item(ui, name, "") {
                    insert(app, *c);
                    ui.close();
                }
            }
        });
        if menu_item(ui, tr("Select slot"), "") {
            app.run("select.set", json!({"ids": [slot.0]}));
            ui.close();
        }
        if edited && menu_item(ui, tr("Reset slot"), "") {
            app.run("instance.resetSlot", json!({"id": slot.0}));
            ui.close();
        }
    });
}

/// Properties of a component or component set.
fn properties_ui(app: &mut FigApp, ui: &mut Ui, doc: &Document, owner: NodeId) {
    let t = Tokens::get(ui.ctx());
    let is_set = matches!(doc.node(owner).map(|n| &n.kind), Some(NodeKind::ComponentSet { .. }));
    // Shared (non-variant) properties live on every variant; read them from the first.
    let first = if is_set {
        doc.node(owner).and_then(|s| s.children.iter().copied().find(|c| matches!(doc.node(*c).map(|n| &n.kind), Some(NodeKind::Component { .. }))))
    } else {
        Some(owner)
    };
    let defs = first.map(|c| prop_list(doc, c)).unwrap_or_default();
    let variants = if is_set { variant_options(doc, owner) } else { Vec::new() };
    let comps = local_components(app, ui, doc);
    widgets::separator(ui);
    let existing: Vec<String> = defs.iter().map(|(k, _)| k.clone()).chain(variants.iter().map(|(k, _)| k.clone())).collect();
    let fresh = (1..).map(|k| format!("Property {k}")).find(|n| !existing.contains(n)).unwrap_or_else(|| "Property".into());
    let plus = header_with_plus(ui, tr("Properties"), tr("Create component property"));
    menu_popup(&plus).width(180.0).show(|ui| {
        let swap_default = comps.iter().map(|(c, _)| *c).find(|c| *c != owner && first != Some(*c));
        let items: [(&str, &str, Value); 4] = [
            (tr("Variant"), "variant", json!("Default")),
            (tr("Boolean"), "boolean", json!(true)),
            (tr("Text"), "text", json!("Text")),
            (tr("Instance swap"), "instanceSwap", swap_default.map(|c| json!(c.0)).unwrap_or(Value::Null)),
        ];
        for (label, ty, default) in items {
            if ty == "instanceSwap" && default.is_null() {
                ui.label(egui::RichText::new(tr("Instance swap (no other component)")).color(t.text_tertiary));
                continue;
            }
            if menu_item(ui, label, "") {
                app.run("component.addProperty", json!({"id": owner, "name": fresh, "type": ty, "default": default}));
                ui.close();
            }
        }
    });
    if defs.is_empty() && variants.is_empty() {
        widgets::note(ui, tr("Add properties to make instances configurable"), t.text_tertiary);
    }
    if is_set && !variant_conflicts(doc, owner).is_empty() {
        widgets::note(ui, tr("Some variants have the same property values: give each one a unique combination"), t.warning);
    }
    let defaults = if is_set {
        default_variant(doc, owner).and_then(|d| doc.node(d)).map(|n| variant_props(&n.name)).unwrap_or_default()
    } else {
        Default::default()
    };
    // Variant properties: name, values (the matrix), add value, delete. Dragged by their left
    // margin to reorder them (only among themselves: the shared ones keep the engine's order).
    let mut drag = ListDrag::in_order(ui, ("variant_props", owner));
    for (i, (k, vals)) in variants.iter().enumerate() {
        let header = row_rect(ui, 32.0, |ui| {
            let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            icons::paint(ui.painter(), ir, Icon::ComponentSet, COMPONENT);
            if let Some(nn) = widgets::text_field(ui, Id::unique(("vprop_name", owner, k.as_str())), k, 110.0, tr("Name")) {
                app.run("component.editProperty", json!({"id": owner, "name": k, "newName": nn}));
            }
            let def = defaults.get(k).or(vals.first()).cloned().unwrap_or_default();
            let resp = widgets::dropdown(ui, &def, 70.0, None).on_hover_text(tr("Default value (the top-left variant)"));
            menu_popup(&resp).show(|ui| {
                for v in vals {
                    if menu_item(ui, v, "") {
                        app.run("component.editProperty", json!({"id": owner, "name": k, "default": v}));
                        ui.close();
                    }
                }
            });
            if icons::button(ui, Icon::Minus, 22.0, false, tr("Delete property")).clicked() {
                app.run("component.deleteProperty", json!({"id": owner, "name": k}));
            }
        });
        // Values: rename inline; "+" adds a variant with a new value.
        let mut vi = 0;
        while vi < vals.len() {
            let chunk: Vec<String> = vals.iter().skip(vi).take(3).cloned().collect();
            let last = vi + chunk.len() >= vals.len();
            row(ui, 30.0, |ui| {
                ui.add_space(22.0);
                for v in &chunk {
                    if let Some(nv) = widgets::text_field(ui, Id::unique(("vprop_val", owner, k.as_str(), v.as_str())), v, 62.0, "")
                        && nv != *v
                    {
                        app.run("component.editProperty", json!({"id": owner, "name": k, "value": v, "newValue": nv}));
                    }
                }
                if last && icons::button(ui, Icon::Plus, 22.0, false, tr("Add variant with a new value")).clicked() {
                    let nv = (vals.len() + 1..).map(|n| format!("Value {n}")).find(|n| !vals.contains(n)).unwrap_or_default();
                    app.run("component.addVariant", json!({"id": owner, "property": k, "value": nv}));
                }
            });
            vi += 3;
        }
        drag.row(ui, i, header, header.with_max_y(ui.cursor().min.y.max(header.max.y)), || k.clone());
    }
    if let Some((from, to)) = drag.finish(ui)
        && let Some((k, _)) = variants.get(from)
    {
        app.run("component.editProperty", json!({"id": owner, "name": k, "index": to}));
    }
    // Shared properties.
    for (k, d) in &defs {
        if matches!(d, PropDef::Variant { .. }) {
            continue;
        }
        row(ui, 32.0, |ui| {
            let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            icons::paint(ui.painter(), ir, type_icon(d), COMPONENT);
            if let Some(nn) = widgets::text_field(ui, Id::unique(("prop_name", owner, k.as_str())), k, 96.0, tr("Name")) {
                app.run("component.editProperty", json!({"id": owner, "name": k, "newName": nn}));
            }
            match d {
                PropDef::Boolean { default } => {
                    if widgets::checkbox(ui, *default, if *default { tr("True") } else { tr("False") }) {
                        app.run("component.editProperty", json!({"id": owner, "name": k, "default": !default}));
                    }
                }
                PropDef::Text { default } => {
                    if let Some(v) = widgets::text_field(ui, Id::unique(("prop_def", owner, k.as_str())), default, 84.0, tr("Default")) {
                        app.run("component.editProperty", json!({"id": owner, "name": k, "default": v}));
                    }
                }
                PropDef::InstanceSwap { default, .. } => {
                    let label = doc.node(*default).map(|n| n.name.clone()).unwrap_or_else(|| tr("Missing").into());
                    let resp = widgets::dropdown(ui, &label, 84.0, None);
                    crate::a11y::combo(&resp, k, &label, crate::a11y::menu_open(&resp));
                    // A file with many components (an icon library) scrolls instead of running off the screen.
                    let all = || vec![(None, all_components(&comps))];
                    if let Some(c) = component_menu(ui, &resp, 200.0, all, |c| c == *default).filter(|c| c != default) {
                        app.run("component.editProperty", json!({"id": owner, "name": k, "default": c.0}));
                    }
                }
                PropDef::Variant { .. } | PropDef::Slot { .. } => {}
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, Icon::Minus, 22.0, false, tr("Delete property")).clicked() {
                    app.run("component.deleteProperty", json!({"id": owner, "name": k}));
                }
            });
        });
        // An instance swap or slot property's preferred components, on a row of their own.
        if let PropDef::Slot { preferred } | PropDef::InstanceSwap { preferred, .. } = d {
            let tip = if matches!(d, PropDef::Slot { .. }) {
                tr("Offered first when filling the slot (any content is allowed)")
            } else {
                tr("Offered first when swapping the instance (any component is allowed)")
            };
            row(ui, 30.0, |ui| {
                ui.add_space(22.0);
                let label = match preferred.len() {
                    0 => tr("No preferred components").to_string(),
                    k => trn("{n} preferred components", k, &[]),
                };
                let w = (ui.available_width() - 8.0).max(60.0);
                let resp = widgets::dropdown(ui, &label, w, None).on_hover_text(tip);
                menu_popup(&resp).width(w.max(200.0)).show(|ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for (c, name) in comps.iter().filter(|(c, _)| *c != owner && first != Some(*c)) {
                            let on = preferred.contains(c);
                            if menu_item(ui, &format!("{}{name}", if on { "✓ " } else { "" }), "") {
                                let next: Vec<u64> = preferred.iter().filter(|p| *p != c).map(|p| p.0).chain((!on).then_some(c.0)).collect();
                                app.run("component.editProperty", json!({"id": owner, "name": k, "preferred": next}));
                            }
                        }
                    });
                });
            });
        }
    }
    // A lone main component too: Add variant makes it a set of itself and the new variant.
    if can_add_variant(doc, owner) {
        row(ui, 32.0, |ui| {
            if widgets::dropdown(ui, tr("Add variant"), 120.0, Some(Icon::Plus)).clicked() {
                app.run("component.addVariant", json!({"id": owner}));
            }
        });
    }
}

/// Bind a layer inside a component to a component property.
fn bind_ui(app: &mut FigApp, ui: &mut Ui, doc: &Document, n: &Node) {
    let t = Tokens::get(ui.ctx());
    let Some(comp) = owning_component(doc, n.id) else { return };
    let defs = prop_list(doc, comp);
    widgets::separator(ui);
    row(ui, 36.0, |ui| {
        ui.label(egui::RichText::new(tr("Component property")).font(semibold(11.0)).color(t.text));
    });
    for field in bind_fields(n) {
        bind_row(app, ui, n, &defs, field);
    }
    slot_name_row(app, ui, n);
}

/// A field of a layer a component property can control: (field, label, property type). The
/// English label also names a new property (document content); it shows translated.
type BindField = (&'static str, &'static str, &'static str);

/// The fields of layer `n` that component properties can control.
fn bind_fields(n: &Node) -> Vec<BindField> {
    let mut fields: Vec<BindField> = vec![("visible", tr_noop("Visibility"), "boolean")];
    if n.kind.text().is_some() {
        fields.push(("characters", tr_noop("Text"), "text"));
    }
    if matches!(n.kind, NodeKind::Instance { .. }) {
        fields.push(("mainComponent", tr_noop("Instance"), "instanceSwap"));
    }
    if matches!(n.kind, NodeKind::Frame) {
        fields.push(("slot", tr_noop("Slot"), "slot"));
    }
    fields
}

/// `base`, or `base 2`, `base 3`…: the first name no property in `defs` has.
fn fresh_name(base: &str, defs: &[(String, PropDef)]) -> String {
    (1..).map(|k| if k == 1 { base.to_string() } else { format!("{base} {k}") }).find(|x| !defs.iter().any(|(k, _)| k == x)).unwrap_or_default()
}

/// The row of one field of layer `n`: its icon, its label and a menu that binds it to a property
/// of type `ty` (or makes one).
fn bind_row(app: &mut FigApp, ui: &mut Ui, n: &Node, defs: &[(String, PropDef)], field: BindField) {
    let t = Tokens::get(ui.ctx());
    let (key, label, _) = field;
    let bound = n.prop_refs.get(key).cloned();
    row(ui, 32.0, |ui| {
        let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        let icon = match key {
            "visible" => Icon::Eye,
            "characters" => Icon::Text,
            "slot" => Icon::Slot,
            _ => Icon::Instance,
        };
        icons::paint(ui.painter(), ir, icon, if bound.is_some() { COMPONENT } else { t.icon_secondary });
        widgets::label(ui, tr(label), t.text_secondary);
        let shown = bound.clone().unwrap_or_else(|| tr("Not bound").into());
        let resp = widgets::dropdown(ui, &shown, 120.0, None).on_hover_text(tr("Apply a component property"));
        crate::a11y::combo(&resp, tr(label), &shown, crate::a11y::menu_open(&resp));
        menu_popup(&resp).width(180.0).show(|ui| bind_menu(app, ui, n, defs, field, bound.is_some()));
    });
}

/// The menu of a field's row ([`bind_row`]): the properties of its type, then making one (a slot
/// is named first, in [`slot_name_row`]) and unbinding (removing the slot).
fn bind_menu(app: &mut FigApp, ui: &mut Ui, n: &Node, defs: &[(String, PropDef)], (key, label, ty): BindField, bound: bool) {
    let fits = |d: &PropDef| {
        matches!(
            (ty, d),
            ("boolean", PropDef::Boolean { .. })
                | ("text", PropDef::Text { .. })
                | ("instanceSwap", PropDef::InstanceSwap { .. })
                | ("slot", PropDef::Slot { .. })
        )
    };
    for (k, _) in defs.iter().filter(|(_, d)| fits(d)) {
        if menu_item(ui, k, "") {
            app.run("component.bindProperty", json!({"layer": n.id, "property": k, "field": key}));
            ui.close();
        }
    }
    let slot = ty == "slot";
    if slot && !bound && menu_item(ui, tr("Make slot"), "") {
        let base = if n.name.trim().is_empty() { label } else { n.name.as_str() };
        ui.data_mut(|d| d.insert_temp(slot_naming_id(n.id), (fresh_name(base, defs), true)));
        ui.close();
    }
    if !slot && menu_item(ui, tr("Create property…"), "") {
        app.run("component.addProperty", json!({"id": n.id, "name": fresh_name(label, defs), "type": ty}));
        ui.close();
    }
    if slot && bound && menu_item(ui, tr("Remove slot"), "") {
        app.run("component.removeSlot", json!({"id": n.id}));
        ui.close();
    }
    if !slot && bound && menu_item(ui, tr("Detach property"), "") {
        app.run("component.bindProperty", json!({"layer": n.id, "property": null, "field": key}));
        ui.close();
    }
}

/// Where Make slot keeps the name being typed for frame `id` (and whether its field takes the
/// keyboard next frame).
fn slot_naming_id(id: NodeId) -> Id {
    Id::unique(("make_slot_name", id))
}

/// Make slot's naming step, once it was picked for frame `n`: a name field that takes the keyboard
/// with the suggested name selected, and a Make slot button. Enter or the button makes the slot
/// with that name; Escape cancels.
fn slot_name_row(app: &mut FigApp, ui: &mut Ui, n: &Node) {
    let id = slot_naming_id(n.id);
    let Some((mut name, mut focus)) = ui.data(|d| d.get_temp::<(String, bool)>(id)) else { return };
    let mut open = !n.prop_refs.contains_key("slot");
    row(ui, 32.0, |ui| {
        ui.add_space(22.0);
        let w = (ui.available_width() - 96.0).max(60.0);
        let field = id.with("field");
        // Asked for until it has it: the click that picked Make slot takes the focus away again.
        if focus && !crate::a11y::field_focused(ui.ctx()) {
            ui.memory_mut(|m| m.request_focus(field));
            widgets::select_all_next(ui, field, &name);
        }
        let r = widgets::text_input(ui, field, &mut name, w, tr("Slot name"));
        crate::a11y::text(&r, tr("Slot name"), &name);
        focus &= !r.has_focus();
        let ok = !name.trim().is_empty();
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let make = widgets::button(ui, tr("Make slot"), true, ok).clicked() || (enter && ok);
        // A refused name (one taken) keeps the field open to fix it.
        if make && app.run("component.makeSlot", json!({"id": n.id, "name": name.trim()})).is_some() {
            open = false;
        }
        if (r.has_focus() || r.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }
    });
    ui.data_mut(|d| {
        if open {
            d.insert_temp(id, (name, focus));
        } else {
            d.remove::<(String, bool)>(id);
        }
    });
}

/// Overridden property groups of a layer inside an instance, each with a reset button.
fn overrides_ui(app: &mut FigApp, ui: &mut Ui, n: &Node) {
    let t = Tokens::get(ui.ctx());
    let groups: Vec<String> = n.overrides.iter().filter(|g| figcraft_engine::components::OVERRIDE_GROUPS.contains(&g.as_str())).cloned().collect();
    if groups.is_empty() {
        return;
    }
    widgets::separator(ui);
    row(ui, 36.0, |ui| {
        ui.label(egui::RichText::new(tr("Overrides")).font(semibold(11.0)).color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::button(ui, Icon::Reset, 24.0, false, tr("Reset all overrides on this layer")).clicked() {
                app.run("component.resetOverride", json!({"id": n.id}));
            }
        });
    });
    for g in groups {
        row(ui, 26.0, |ui| {
            widgets::label(ui, &group_label(&g), t.text_secondary);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, Icon::Close, 20.0, false, tr("Reset this override")).clicked() {
                    app.run("component.resetOverride", json!({"id": n.id, "group": g}));
                }
            });
        });
    }
}

fn group_label(g: &str) -> String {
    match g {
        "fills" => tr("Fill"),
        "strokes" => tr("Stroke"),
        "effects" => tr("Effects"),
        "opacity" => tr("Opacity & blend"),
        "visible" => tr("Visibility"),
        "name" => tr("Layer name"),
        "corners" => tr("Corner radius"),
        "characters" => tr("Text content"),
        "textStyle" => tr("Text style"),
        "padding" => tr("Padding"),
        "gap" => tr("Gap"),
        "alignment" => tr("Alignment"),
        "grids" => tr("Layout grids"),
        "exports" => tr("Export settings"),
        "interactions" => tr("Interactions"),
        "modes" => tr("Variable modes"),
        "size" => tr("Size"),
        "resizing" => tr("Resizing"),
        "position" => tr("Position"),
        "component" => tr("Swapped instance"),
        "props" => tr("Instance properties"),
        "slot" => tr("Slot content"),
        other => other,
    }
    .to_string()
}

/// The instance section: main component + swap, variants, properties, actions.
fn instance_ui(app: &mut FigApp, ui: &mut Ui, doc: &Document, n: &Node) {
    let t = Tokens::get(ui.ctx());
    let NodeKind::Instance { component, .. } = &n.kind else { return };
    let id = n.id;
    let missing = !doc.contains(*component);
    let set = variant_set(doc, *component);
    let comp_name = set.or(Some(*component)).and_then(|c| doc.node(c)).map(|c| c.name.clone()).unwrap_or_else(|| tr("Main component deleted").into());
    let comps = local_components(app, ui, doc);
    let nested = inside_instance(doc, id);
    widgets::separator(ui);
    row(ui, 36.0, |ui| {
        ui.label(egui::RichText::new(if nested { tr("Nested instance") } else { tr("Instance") }).font(semibold(11.0)).color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let more = icons::button(ui, Icon::Dots, 24.0, false, tr("More instance actions"));
            menu_popup(&more).width(220.0).show(|ui| {
                if !missing && menu_item(ui, tr("Go to main component"), "") {
                    app.run("component.goToMain", json!({"id": id}));
                    ui.close();
                }
                if !missing && menu_item(ui, tr("Push changes to main component"), "") {
                    app.run("component.pushChanges", json!({"id": id}));
                    ui.close();
                }
                if missing && !nested && menu_item(ui, tr("Restore main component"), "") {
                    app.run("component.restoreMain", json!({"id": id}));
                    ui.close();
                }
                if menu_item(ui, tr("Reset all changes"), "") {
                    app.run("component.resetInstance", json!({"ids": [id]}));
                    ui.close();
                }
                if !nested && menu_item(ui, tr("Detach instance"), "Alt+Cmd+B") {
                    app.run("component.detach", json!({"ids": [id]}));
                    ui.close();
                }
            });
            if !nested && icons::button(ui, Icon::Detach, 24.0, false, crate::shortcuts::tip(tr("Detach instance"), "component.detach")).clicked() {
                app.run("component.detach", json!({"ids": [id]}));
            }
            if icons::button(ui, Icon::Reset, 24.0, false, tr("Reset all changes")).clicked() {
                app.run("component.resetInstance", json!({"ids": [id]}));
            }
        });
    });
    // Main component with a swap menu, and "go to main".
    row(ui, 32.0, |ui| {
        let w = (ui.available_width() - 32.0).max(80.0);
        let resp = widgets::dropdown(ui, &comp_name, w, Some(Icon::Instance)).on_hover_text(tr("Swap instance"));
        let current = |c: NodeId| Some(c) == set || c == *component;
        if let Some(c) = component_menu(ui, &resp, w.max(200.0), || by_page(doc, &comps), current).filter(|c| !current(*c)) {
            app.run("instance.swap", json!({"ids": [id], "component": c.0}));
        }
        if missing {
            if icons::button(ui, Icon::Component, 24.0, false, tr("Restore main component")).clicked() {
                app.run("component.restoreMain", json!({"id": id}));
            }
        } else if icons::button(ui, Icon::Component, 24.0, false, tr("Go to main component")).clicked() {
            app.run("component.goToMain", json!({"id": id}));
        }
    });
    // The main component's description (the set's when the variant has none).
    let desc = [Some(*component), set]
        .into_iter()
        .flatten()
        .filter_map(|c| match doc.node(c).map(|n| &n.kind) {
            Some(NodeKind::Component { description, .. } | NodeKind::ComponentSet { description }) => Some(description.trim().to_string()),
            _ => None,
        })
        .find(|d| !d.is_empty());
    if let Some(d) = desc {
        ui.horizontal(|ui| {
            ui.add_space(crate::widgets::PAD);
            ui.add(egui::Label::new(egui::RichText::new(d).font(crate::theme::regular(11.0)).color(t.text_secondary)).wrap());
        });
        ui.add_space(4.0);
    }
    // A nested instance inside a main component can expose its properties on the main's instances.
    if nested_in_main(doc, id) {
        row(ui, 32.0, |ui| {
            if widgets::checkbox(ui, n.exposed, tr("Expose properties to instances")) {
                app.run("component.exposeInstance", json!({"id": id, "exposed": !n.exposed}));
            }
        });
    }
    instance_props(app, ui, doc, n, &comps);
    // Exposed nested instances: their properties, under their names.
    if !nested {
        for c in figcraft_engine::components::exposed_copies(doc, id) {
            let Some(cn) = doc.node(c) else { continue };
            row(ui, 28.0, |ui| {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Instance, crate::theme::COMPONENT);
                ui.label(egui::RichText::new(&cn.name).font(semibold(11.0)).color(t.text));
            });
            instance_props(app, ui, doc, cn, &comps);
        }
    }
}

/// An instance placed inside a main component (not inside another instance).
fn nested_in_main(doc: &Document, id: NodeId) -> bool {
    !inside_instance(doc, id) && figcraft_engine::components::owning_component(doc, id).is_some_and(|c| c != id)
}

/// Variant and component property controls for instance `n`.
fn instance_props(app: &mut FigApp, ui: &mut Ui, doc: &Document, n: &Node, comps: &[(NodeId, String)]) {
    let NodeKind::Instance { component, props } = &n.kind else { return };
    let id = n.id;
    let set = variant_set(doc, *component);
    let slots = slots_of(doc, id);
    // Variant properties.
    if let Some(set) = set {
        let current = doc.node(*component).map(|c| variant_props(&c.name)).unwrap_or_default();
        for (k, vals) in variant_options(doc, set) {
            let cur = current.get(&k).cloned().unwrap_or_default();
            row(ui, 32.0, |ui| {
                prop_label(ui, &k, Icon::ComponentSet);
                let w = ui.available_width().max(60.0);
                // A string variable naming one of the values can choose it.
                crate::variable_ui::bindable(app, ui, &[id], &format!("{PROP_BINDING}{k}"), VariableType::String, w, |app, ui| {
                    let resp = widgets::dropdown(ui, &cur, w, None);
                    menu_popup(&resp).show(|ui| {
                        for v in &vals {
                            if menu_item(ui, v, "") {
                                app.run("instance.setProperties", json!({"ids": [id], "properties": {k.clone(): v}}));
                                ui.close();
                            }
                        }
                    });
                });
            });
        }
    }
    // Boolean, text and instance-swap properties.
    for (k, d) in prop_list(doc, *component) {
        let v = props.get(&k).cloned().unwrap_or_else(|| figcraft_engine::components::prop_default_value(&d));
        row(ui, 32.0, |ui| {
            prop_label(ui, &k, type_icon(&d));
            let w = ui.available_width().max(60.0);
            // Boolean and text values can come from a boolean or string variable.
            let bind_field = format!("{PROP_BINDING}{k}");
            match d {
                PropDef::Boolean { .. } => {
                    crate::variable_ui::bindable(app, ui, &[id], &bind_field, VariableType::Boolean, w, |app, ui| {
                        let on = v.as_bool().unwrap_or(false);
                        if widgets::checkbox(ui, on, "") {
                            app.run("instance.setProperties", json!({"ids": [id], "properties": {k.clone(): !on}}));
                        }
                    });
                }
                PropDef::Text { .. } => {
                    crate::variable_ui::bindable(app, ui, &[id], &bind_field, VariableType::String, w, |app, ui| {
                        let cur = v.as_str().unwrap_or_default().to_string();
                        if let Some(ns) = widgets::text_field(ui, Id::unique(("iprop", id, k.as_str())), &cur, w, "")
                            && ns != cur
                        {
                            app.run("instance.setProperties", json!({"ids": [id], "properties": {k.clone(): ns}}));
                        }
                    });
                }
                PropDef::InstanceSwap { preferred, .. } => {
                    if let Some(c) = swap_dropdown(ui, doc, figcraft_engine::params::id_of(&v), &preferred, comps, w, &k).1 {
                        app.run("instance.setProperties", json!({"ids": [id], "properties": {k.clone(): c.0}}));
                    }
                }
                PropDef::Variant { .. } => {}
                PropDef::Slot { preferred } => {
                    if let Some((slot, _)) = slots.iter().find(|(_, name)| *name == k) {
                        slot_fill(app, ui, doc, *slot, &preferred, comps, w);
                    }
                }
            }
        });
    }
}

/// The dropdown of an instance-swap property showing component `cur`: its menu lists the
/// property's preferred components first (when it has any), then every component in the file. The
/// dropdown, and the component picked (picking the current one picks nothing). `name` labels it for
/// assistive technology.
pub(crate) fn swap_dropdown(
    ui: &mut Ui,
    doc: &Document,
    cur: Option<NodeId>,
    preferred: &[NodeId],
    comps: &[(NodeId, String)],
    w: f32,
    name: &str,
) -> (egui::Response, Option<NodeId>) {
    let label = cur.and_then(|c| doc.node(c)).map(|c| c.name.clone()).unwrap_or_else(|| tr("None").into());
    let resp = widgets::dropdown(ui, &label, w, Some(Icon::Instance));
    if !name.is_empty() {
        crate::a11y::combo(&resp, name, &label, crate::a11y::menu_open(&resp));
    }
    let groups = || {
        let preferred: Vec<(NodeId, &str)> = preferred.iter().filter_map(|c| doc.node(*c).map(|n| (*c, n.name.as_str()))).collect();
        if preferred.is_empty() {
            vec![(None, all_components(comps))]
        } else {
            vec![(Some(tr("Preferred")), preferred), (Some(tr("All components")), all_components(comps))]
        }
    };
    // Picking the current component picks nothing.
    let picked = component_menu(ui, &resp, 200.0, groups, |c| cur == Some(c)).filter(|c| cur != Some(*c));
    (resp, picked)
}

/// A component menu's groups: a heading (or none) and its components (id, name).
type CompGroups<'a> = Vec<(Option<&'a str>, Vec<(NodeId, &'a str)>)>;

/// `comps` under their pages' names (they come page by page).
fn by_page<'a>(doc: &'a Document, comps: &'a [(NodeId, String)]) -> CompGroups<'a> {
    let mut groups: CompGroups = Vec::new();
    let mut page = None;
    for (c, name) in comps {
        let pg = doc.page_of(*c);
        if groups.is_empty() || pg != page {
            page = pg;
            groups.push((pg.and_then(|p| doc.node(p)).map(|p| p.name.as_str()), Vec::new()));
        }
        if let Some((_, cs)) = groups.last_mut() {
            cs.push((*c, name.as_str()));
        }
    }
    groups
}

/// `comps` as one group's components.
fn all_components(comps: &[(NodeId, String)]) -> Vec<(NodeId, &str)> {
    comps.iter().map(|(c, n)| (*c, n.as_str())).collect()
}

/// The tallest a component menu is, its search field included; its list scrolls past that.
const COMP_MENU_HEIGHT: f32 = 360.0;

/// What a component menu's search found this frame.
struct CompSearch<'a> {
    /// The groups with the components whose names match (groups left empty dropped).
    groups: CompGroups<'a>,
    /// The match Up and Down are on (counted across the groups), and whether they moved it now.
    hi: Option<usize>,
    moved: bool,
    /// The component Enter picked.
    entered: Option<NodeId>,
}

/// The menu `field` opens, at least `width` wide, listing `groups` (made only while it is open)
/// under a search field that takes the keyboard when the menu opens and filters the components by
/// name as you type. Up and Down step through the matches and Enter picks the one they are on (or
/// the first); Escape clears the search, then closes the menu. `checked` components are ticked.
/// The component picked.
fn component_menu<'a>(
    ui: &Ui,
    field: &egui::Response,
    width: f32,
    groups: impl FnOnce() -> CompGroups<'a>,
    checked: impl Fn(NodeId) -> bool,
) -> Option<NodeId> {
    let opened = field.clicked();
    craft_ui::combo::Menu::new(ui, field)
        .height(COMP_MENU_HEIGHT)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .style(crate::right::menu_style)
        .show_ui_with_header(
            |ui| comp_search(ui, field.id, width, opened, groups()),
            |ui, found| found.entered.or_else(|| comp_rows(ui, &found, &checked)),
        )
        .flatten()
}

/// A component menu's search field over `groups`, and what it matches.
fn comp_search<'a>(ui: &mut Ui, field: Id, width: f32, opened: bool, groups: CompGroups<'a>) -> CompSearch<'a> {
    ui.set_min_width(width);
    let state = field.with("comp_search");
    let (mut q, mut hi): (String, Option<usize>) = if opened { Default::default() } else { ui.data(|d| d.get_temp(state)).unwrap_or_default() };
    let before = q.clone();
    // Taken before the search field sees them (it would move its cursor, or give up the keyboard).
    let (step, enter) = ui.input_mut(|i| {
        if !q.is_empty() && i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
            q.clear();
        }
        let down = i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) as i64;
        (down - i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) as i64, i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
    });
    let r = ui.add(
        egui::TextEdit::singleline(&mut q)
            .id(field.with("comp_query"))
            // Escape clears the search (above) and keeps the keyboard here; with nothing to clear it closes the menu.
            .event_filter(egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, tab: false, escape: true })
            .hint_text(tr("Search"))
            .desired_width(ui.available_width())
            .font(crate::theme::regular(11.5)),
    );
    widgets::select_all_on_focus(ui, &r, &q);
    if opened {
        r.request_focus();
    }
    let ql = q.trim().to_lowercase();
    let groups: CompGroups<'a> = groups
        .into_iter()
        .map(|(h, cs)| (h, cs.into_iter().filter(|(_, n)| n.to_lowercase().contains(&ql)).collect::<Vec<_>>()))
        .filter(|(_, cs)| !cs.is_empty())
        .collect();
    let n = groups.iter().map(|(_, cs)| cs.len()).sum::<usize>();
    // A new search (or one cleared) starts again from the top.
    if q != before || n == 0 {
        hi = None;
    }
    let moved = step != 0 && n > 0;
    if moved {
        let at = hi.map_or(if step > 0 { -1 } else { n as i64 }, |h| h as i64) + step;
        hi = usize::try_from(at.clamp(0, n as i64 - 1)).ok();
    }
    let entered = enter.then(|| groups.iter().flat_map(|(_, cs)| cs).nth(hi.unwrap_or(0)).map(|(c, _)| *c)).flatten();
    if entered.is_some() {
        ui.close();
    }
    ui.data_mut(|d| d.insert_temp(state, (q, hi)));
    CompSearch { groups, hi, moved, entered }
}

/// A component menu's rows for what its search `found` (the one Up and Down are on lit); the
/// component clicked.
fn comp_rows(ui: &mut Ui, found: &CompSearch, checked: &impl Fn(NodeId) -> bool) -> Option<NodeId> {
    let t = Tokens::get(ui.ctx());
    if found.groups.is_empty() {
        widgets::label(ui, tr("No matching components"), t.text_tertiary);
        return None;
    }
    let mut picked = None;
    let mut k = 0;
    for (heading, comps) in &found.groups {
        if let Some(h) = heading {
            ui.label(egui::RichText::new(*h).font(semibold(11.0)).color(t.text_secondary));
        }
        for (c, name) in comps {
            let lit = found.hi == Some(k);
            k += 1;
            let label = if checked(*c) { format!("✓ {name}") } else { (*name).to_string() };
            let resp = crate::right::menu_row_lit(ui, &label, lit);
            if lit && found.moved {
                resp.scroll_to_me(None);
            }
            if resp.clicked() {
                picked = Some(*c);
                ui.close();
            }
        }
    }
    picked
}

fn prop_label(ui: &mut Ui, name: &str, icon: Icon) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(84.0, 20.0), Sense::hover());
    icons::paint(ui.painter(), egui::Rect::from_min_size(egui::pos2(r.min.x, r.center().y - 6.0), vec2(12.0, 12.0)), icon, t.icon_secondary);
    let g = ui.painter().layout(name.to_string(), crate::theme::regular(11.0), t.text_secondary, r.width() - 18.0);
    ui.painter().galley(egui::pos2(r.min.x + 18.0, r.center().y - g.size().y / 2.0), g, t.text_secondary);
}
