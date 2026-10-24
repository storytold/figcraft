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
use figcraft_doc::*;
use figcraft_engine::cmd::component::{can_add_variant, can_combine_variants};
use figcraft_engine::components::{
    PROP_BINDING, default_variant, inside_instance, owning_component, owning_instance, slot_def, slot_edited, slots_of, variant_conflicts,
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
    if let Some(new) = widgets::multiline_field(ui, Id::new(("comp_desc", id)), description, tr("Describe how and when to use this component"), 3) {
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
    let id = Id::new("component_panel_local_components");
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

fn prop_defs(doc: &Document, c: NodeId) -> std::collections::BTreeMap<String, PropDef> {
    match doc.node(c).map(|n| &n.kind) {
        Some(NodeKind::Component { props, .. }) => props.clone(),
        _ => Default::default(),
    }
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
    let defs = first.map(|c| prop_defs(doc, c)).unwrap_or_default();
    let variants = if is_set { variant_options(doc, owner) } else { Vec::new() };
    let comps = local_components(app, ui, doc);
    widgets::separator(ui);
    let existing: Vec<String> = defs.keys().cloned().chain(variants.iter().map(|(k, _)| k.clone())).collect();
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
            if let Some(nn) = widgets::text_field(ui, Id::new(("vprop_name", owner, k.as_str())), k, 110.0, tr("Name")) {
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
                    if let Some(nv) = widgets::text_field(ui, Id::new(("vprop_val", owner, k.as_str(), v.as_str())), v, 62.0, "")
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
            if let Some(nn) = widgets::text_field(ui, Id::new(("prop_name", owner, k.as_str())), k, 96.0, tr("Name")) {
                app.run("component.editProperty", json!({"id": owner, "name": k, "newName": nn}));
            }
            match d {
                PropDef::Boolean { default } => {
                    if widgets::checkbox(ui, *default, if *default { tr("True") } else { tr("False") }) {
                        app.run("component.editProperty", json!({"id": owner, "name": k, "default": !default}));
                    }
                }
                PropDef::Text { default } => {
                    if let Some(v) = widgets::text_field(ui, Id::new(("prop_def", owner, k.as_str())), default, 84.0, tr("Default")) {
                        app.run("component.editProperty", json!({"id": owner, "name": k, "default": v}));
                    }
                }
                PropDef::InstanceSwap { default, .. } => {
                    let label = doc.node(*default).map(|n| n.name.clone()).unwrap_or_else(|| tr("Missing").into());
                    let resp = widgets::dropdown(ui, &label, 84.0, None);
                    menu_popup(&resp).show(|ui| {
                        for (c, name) in comps.iter() {
                            if menu_item(ui, name, "") {
                                app.run("component.editProperty", json!({"id": owner, "name": k, "default": c.0}));
                                ui.close();
                            }
                        }
                    });
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
    let defs = prop_defs(doc, comp);
    // The English label also names a new property (document content); it shows translated.
    let mut fields: Vec<(&str, &str, &str)> = vec![("visible", tr_noop("Visibility"), "boolean")];
    if n.kind.text().is_some() {
        fields.push(("characters", tr_noop("Text"), "text"));
    }
    if matches!(n.kind, NodeKind::Instance { .. }) {
        fields.push(("mainComponent", tr_noop("Instance"), "instanceSwap"));
    }
    if matches!(n.kind, NodeKind::Frame) {
        fields.push(("slot", tr_noop("Slot"), "slot"));
    }
    widgets::separator(ui);
    row(ui, 36.0, |ui| {
        ui.label(egui::RichText::new(tr("Component property")).font(semibold(11.0)).color(t.text));
    });
    for (field, label, ty) in fields {
        let matching: Vec<&String> = defs
            .iter()
            .filter(|(_, d)| {
                matches!(
                    (ty, d),
                    ("boolean", PropDef::Boolean { .. })
                        | ("text", PropDef::Text { .. })
                        | ("instanceSwap", PropDef::InstanceSwap { .. })
                        | ("slot", PropDef::Slot { .. })
                )
            })
            .map(|(k, _)| k)
            .collect();
        let bound = n.prop_refs.get(field).cloned();
        row(ui, 32.0, |ui| {
            let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            let icon = match field {
                "visible" => Icon::Eye,
                "characters" => Icon::Text,
                "slot" => Icon::Slot,
                _ => Icon::Instance,
            };
            icons::paint(ui.painter(), ir, icon, if bound.is_some() { COMPONENT } else { t.icon_secondary });
            widgets::label(ui, tr(label), t.text_secondary);
            let shown = bound.clone().unwrap_or_else(|| tr("Not bound").into());
            let resp = widgets::dropdown(ui, &shown, 120.0, None).on_hover_text(tr("Apply a component property"));
            menu_popup(&resp).width(180.0).show(|ui| {
                for k in &matching {
                    if menu_item(ui, k, "") {
                        app.run("component.bindProperty", json!({"layer": n.id, "property": k, "field": field}));
                        ui.close();
                    }
                }
                if ty == "slot" {
                    if bound.is_none() && menu_item(ui, tr("Make slot"), "") {
                        // Named after the frame.
                        app.run("component.makeSlot", json!({"id": n.id}));
                        ui.close();
                    }
                } else if menu_item(ui, tr("Create property…"), "") {
                    let existing: Vec<String> = defs.keys().cloned().collect();
                    let name = (1..)
                        .map(|k| if k == 1 { label.to_string() } else { format!("{label} {k}") })
                        .find(|x| !existing.contains(x))
                        .unwrap_or_default();
                    app.run("component.addProperty", json!({"id": n.id, "name": name, "type": ty}));
                    ui.close();
                }
                if ty == "slot" {
                    if bound.is_some() && menu_item(ui, tr("Remove slot"), "") {
                        app.run("component.removeSlot", json!({"id": n.id}));
                        ui.close();
                    }
                } else if bound.is_some() && menu_item(ui, tr("Detach property"), "") {
                    app.run("component.bindProperty", json!({"layer": n.id, "property": null, "field": field}));
                    ui.close();
                }
            });
        });
    }
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
        menu_popup(&resp).width(w.max(200.0)).show(|ui| {
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                let mut page = None;
                for (c, name) in comps.iter() {
                    let pg = doc.page_of(*c);
                    if pg != page {
                        page = pg;
                        let pname = pg.and_then(|p| doc.node(p)).map(|p| p.name.clone()).unwrap_or_default();
                        ui.label(egui::RichText::new(pname).font(semibold(11.0)).color(t.text_secondary));
                    }
                    let current = Some(*c) == set || *c == *component;
                    if menu_item(ui, &format!("{}{name}", if current { "✓ " } else { "" }), "") && !current {
                        app.run("instance.swap", json!({"ids": [id], "component": c.0}));
                        ui.close();
                    }
                }
            });
        });
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
    for (k, d) in prop_defs(doc, *component) {
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
                        if let Some(ns) = widgets::text_field(ui, Id::new(("iprop", id, k.as_str())), &cur, w, "")
                            && ns != cur
                        {
                            app.run("instance.setProperties", json!({"ids": [id], "properties": {k.clone(): ns}}));
                        }
                    });
                }
                PropDef::InstanceSwap { preferred, .. } => {
                    if let Some(c) = swap_dropdown(ui, doc, figcraft_engine::params::id_of(&v), &preferred, comps, w, "").1 {
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
    let mut picked = None;
    menu_popup(&resp).width(200.0).show(|ui| {
        let t = Tokens::get(ui.ctx());
        let heading = |ui: &mut Ui, text: &str| ui.label(egui::RichText::new(text).font(semibold(11.0)).color(t.text_secondary));
        let mut pick = |ui: &mut Ui, c: NodeId, name: &str| {
            let current = cur == Some(c);
            if menu_item(ui, &format!("{}{name}", if current { "✓ " } else { "" }), "") {
                if !current {
                    picked = Some(c);
                }
                ui.close();
            }
        };
        let preferred: Vec<(NodeId, String)> = preferred.iter().filter_map(|c| doc.node(*c).map(|n| (*c, n.name.clone()))).collect();
        if !preferred.is_empty() {
            heading(ui, tr("Preferred"));
            for (c, name) in &preferred {
                pick(ui, *c, name);
            }
            heading(ui, tr("All components"));
        }
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for (c, name) in comps {
                pick(ui, *c, name);
            }
        });
    });
    (resp, picked)
}

fn prop_label(ui: &mut Ui, name: &str, icon: Icon) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(84.0, 20.0), Sense::hover());
    icons::paint(ui.painter(), egui::Rect::from_min_size(egui::pos2(r.min.x, r.center().y - 6.0), vec2(12.0, 12.0)), icon, t.icon_secondary);
    let g = ui.painter().layout(name.to_string(), crate::theme::regular(11.0), t.text_secondary, r.width() - 18.0);
    ui.painter().galley(egui::pos2(r.min.x + 18.0, r.center().y - g.size().y / 2.0), g, t.text_secondary);
}
