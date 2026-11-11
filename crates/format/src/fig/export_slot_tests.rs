//! `.fig` export round trips of component properties, especially slots: the definitions on a
//! main, the property references on its layers, an instance's property values and the content its
//! slots hold. These are the fields a `.fig` written by the exporter is read back from.

use std::collections::BTreeMap;

use figcraft_doc::{Document, NodeId, NodeKind, PropDef, TextData};
use serde_json::Value;

use super::kiwi::Kv;
use super::{Archive, decode_canvas, export, import};

/// A copy of `src` below `parent`, linked back to it as an instance's expanded layer.
fn mirror(doc: &mut Document, src: NodeId, parent: NodeId) -> NodeId {
    let mut n = doc.node(src).unwrap().clone();
    let kids = std::mem::take(&mut n.children);
    n.id = doc.alloc_id();
    n.source = Some(src);
    let id = doc.insert(n, parent, None).unwrap();
    for k in kids {
        mirror(doc, k, id);
    }
    id
}

/// A "Card" main with a text, a boolean and a slot property. Its slot has default content; the
/// instance overrides every property. With `own_slot`, it also puts its own content in the slot.
fn slots_document_with(own_slot: bool) -> Document {
    let mut doc = Document::new("Slots");
    let page = doc.first_page().unwrap();
    let props = BTreeMap::from([
        ("Label".to_string(), PropDef::Text { default: "Title".into() }),
        ("Visible".to_string(), PropDef::Boolean { default: true }),
        ("Content".to_string(), PropDef::Slot { preferred: Vec::new() }),
    ]);
    let main =
        doc.create(page, NodeKind::Component { description: String::new(), props, prop_order: Vec::new() }, "Card", 0.0, 0.0, 300.0, 160.0).unwrap();
    let label = doc.create(main, NodeKind::Text(TextData::new("Title")), "Label", 0.0, 0.0, 200.0, 20.0).unwrap();
    doc.node_mut(label).unwrap().prop_refs.insert("characters".into(), "Label".into());
    let slot = doc.create(main, NodeKind::Frame, "Content", 0.0, 30.0, 280.0, 100.0).unwrap();
    doc.node_mut(slot).unwrap().prop_refs.insert("slot".into(), "Content".into());
    doc.create(slot, NodeKind::Rectangle, "Default", 0.0, 0.0, 40.0, 20.0).unwrap();

    let id = doc.alloc_id();
    let mut inst = doc.node(main).unwrap().clone();
    let inst_props = BTreeMap::from([("Label".to_string(), Value::String("Hi".into())), ("Visible".to_string(), Value::Bool(false))]);
    (inst.id, inst.kind, inst.name, inst.parent, inst.source) =
        (id, NodeKind::Instance { component: main, props: inst_props }, "Card".into(), None, None);
    inst.children.clear();
    doc.insert(inst, page, None).unwrap();
    let mirrored: Vec<_> = doc.node(main).unwrap().children.clone();
    for k in mirrored {
        let m = mirror(&mut doc, k, id);
        let is_slot = doc.node(m).unwrap().prop_refs.contains_key("slot");
        let is_text = matches!(doc.node(m).unwrap().kind, NodeKind::Text(_));
        let n = doc.node_mut(m).unwrap();
        if is_text {
            if let NodeKind::Text(td) = &mut n.kind {
                td.characters = "Hi".into();
            }
            n.overrides.insert("characters".into());
        }
        // The slot keeps its own content instead of the main's default.
        if own_slot && is_slot {
            n.prop_refs.clear();
            n.overrides.insert("slot".into());
            n.children.clear();
            doc.create(m, NodeKind::Text(TextData::new("Own")), "Own", 0.0, 0.0, 100.0, 20.0).unwrap();
        }
    }
    doc
}

fn slots_document() -> Document {
    slots_document_with(true)
}

fn names(doc: &Document, id: NodeId) -> Vec<String> {
    doc.node(id).unwrap().children.iter().map(|c| doc.node(*c).unwrap().name.clone()).collect()
}

fn instance_slot(doc: &Document) -> &figcraft_doc::Node {
    let inst = doc.nodes().find(|n| matches!(n.kind, NodeKind::Instance { .. })).unwrap();
    inst.children.iter().filter_map(|c| doc.node(*c)).find(|n| n.name == "Content").unwrap()
}

/// A component set whose variants share a slot property, with an instance of the large variant
/// putting its own content in that slot.
fn set_slots_document() -> Document {
    let mut doc = Document::new("Set slots");
    let page = doc.first_page().unwrap();
    let set = doc.create(page, NodeKind::ComponentSet { description: String::new() }, "Panel", 0.0, 0.0, 300.0, 100.0).unwrap();
    let mut variants = Vec::new();
    for (i, name) in ["Size=Small", "Size=Large"].into_iter().enumerate() {
        let props = BTreeMap::from([("Content".to_string(), PropDef::Slot { preferred: Vec::new() })]);
        let v = doc
            .create(set, NodeKind::Component { description: String::new(), props, prop_order: Vec::new() }, name, 0.0, 60.0 * i as f64, 100.0, 100.0)
            .unwrap();
        let slot = doc.create(v, NodeKind::Frame, "Content", 0.0, 0.0, 90.0, 60.0).unwrap();
        doc.node_mut(slot).unwrap().prop_refs.insert("slot".into(), "Content".into());
        variants.push(v);
    }
    let large = variants[1];
    let id = doc.alloc_id();
    let mut inst = doc.node(large).unwrap().clone();
    (inst.id, inst.kind, inst.name, inst.parent, inst.source) =
        (id, NodeKind::Instance { component: large, props: BTreeMap::new() }, "Panel".into(), None, None);
    inst.children.clear();
    doc.insert(inst, page, None).unwrap();
    for k in doc.node(large).unwrap().children.clone() {
        mirror(&mut doc, k, id);
    }
    let slot = doc.node(id).unwrap().children[0];
    let n = doc.node_mut(slot).unwrap();
    n.prop_refs.clear();
    n.overrides.insert("slot".into());
    n.children.clear();
    doc.create(slot, NodeKind::Text(TextData::new("Mine")), "Mine", 0.0, 0.0, 80.0, 20.0).unwrap();
    doc
}

/// The written message carries the SLOT definition, the main layer's SLOT_CONTENT_ID reference,
/// the instance's assignment and its private slot content frame — with the field ids of the real
/// format. No override row may unset the main's bindings.
#[test]
fn slot_fields_are_written_to_the_kiwi_message() {
    let doc = slots_document();
    let (bytes, _) = export(&doc, None).unwrap();
    let archive = Archive::open(&bytes).unwrap();
    let (schema, message) = decode_canvas(&archive.canvas).unwrap();
    let msg = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
    let changes = msg.get("nodeChanges").unwrap().list();

    let main = changes.iter().find(|n| n.get("type").and_then(Kv::str) == Some("SYMBOL")).unwrap();
    let defs = main.get("componentPropDefs").unwrap().list();
    assert_eq!(defs.iter().filter(|d| d.get("type").and_then(Kv::str) == Some("SLOT")).count(), 1);

    let slot_frame = changes.iter().find(|n| n.get("name").and_then(Kv::str) == Some("Content")).unwrap();
    let entries = slot_frame.get("parameterConsumptionMap").unwrap().get("entries").unwrap().list();
    assert!(entries.iter().any(|e| e.get("variableField").and_then(Kv::str) == Some("SLOT_CONTENT_ID")));

    let inst = changes.iter().find(|n| n.get("type").and_then(Kv::str) == Some("INSTANCE")).unwrap();
    let assignments = inst.get("componentPropAssignments").unwrap().list();
    assert!(assignments.iter().any(|a| a.get("varValue").and_then(|v| v.get("dataType")).and_then(Kv::str) == Some("SLOT_CONTENT_ID")));
    assert!(changes.iter().any(|n| n.get("isSlotContent").and_then(Kv::bool) == Some(true)));

    // An "unset" row for parameterConsumptionMap would clear the main's binding on import.
    let overrides = inst.get("symbolData").unwrap().get("symbolOverrides").unwrap().list();
    assert!(!overrides.iter().any(|o| o.get("parameterConsumptionMap").is_some()), "instance rows must inherit the main's property bindings");
}

#[test]
fn component_properties_and_slot_content_round_trip() {
    let doc = slots_document();
    let (bytes, warnings) = export(&doc, None).unwrap();
    let (back, _) = import(&bytes, "Slots").unwrap();

    // The main keeps all three definitions, its text binding and its slot binding.
    let main = back.nodes().find(|n| matches!(n.kind, NodeKind::Component { .. })).unwrap();
    let NodeKind::Component { props, .. } = &main.kind else { panic!("not a component") };
    assert_eq!(props.get("Label"), Some(&PropDef::Text { default: "Title".into() }), "{props:?}");
    assert_eq!(props.get("Visible"), Some(&PropDef::Boolean { default: true }), "{props:?}");
    assert!(matches!(props.get("Content"), Some(PropDef::Slot { .. })), "the slot definition came back: {props:?}");
    let main_slot = main.children.iter().filter_map(|c| back.node(*c)).find(|n| n.prop_refs.contains_key("slot")).unwrap();
    assert_eq!(main_slot.prop_refs.get("slot").map(String::as_str), Some("Content"));
    assert_eq!(names(&back, main_slot.id), ["Default"]);

    // The instance's own values, its slot override and the content it holds.
    let inst = back.nodes().find(|n| matches!(n.kind, NodeKind::Instance { .. })).unwrap();
    let NodeKind::Instance { props, .. } = &inst.kind else { panic!("not an instance") };
    assert_eq!(props.get("Label"), Some(&Value::String("Hi".into())), "{props:?}");
    assert_eq!(props.get("Visible"), Some(&Value::Bool(false)), "{props:?}");
    let slot = inst.children.iter().filter_map(|c| back.node(*c)).find(|n| n.overrides.contains("slot")).unwrap();
    assert_eq!(names(&back, slot.id), ["Own"], "the instance's slot content came back");
    assert!(!warnings.iter().any(|w| w.contains("slot")), "slots should be written, not warned about: {warnings:?}");
}

/// A synced instance keeps no property references on its expanded layers at all. The exporter
/// must not turn that into "unset" rows, which used to clear every binding (slots included).
#[test]
fn instance_layers_inherit_their_mains_property_bindings() {
    let mut doc = slots_document();
    for id in doc.ids().collect::<Vec<_>>() {
        if let Some(n) = doc.node_mut(id)
            && n.source.is_some()
        {
            n.prop_refs.clear();
        }
    }
    let (bytes, warnings) = export(&doc, None).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let (back, _) = import(&bytes, "Slots").unwrap();

    let labels: Vec<_> = back.nodes().filter(|n| n.name == "Label").collect();
    assert_eq!(labels.len(), 2, "the main's and the instance's label");
    assert!(labels.iter().all(|n| n.prop_refs.get("characters").map(String::as_str) == Some("Label")), "both keep the text binding");
    let inst = back.nodes().find(|n| matches!(n.kind, NodeKind::Instance { .. })).unwrap();
    let slot = inst.children.iter().filter_map(|c| back.node(*c)).find(|n| n.overrides.contains("slot")).unwrap();
    assert_eq!(names(&back, slot.id), ["Own"]);
}

/// An instance that doesn't assign its slot keeps the main's default content: no private frame,
/// no assignment, and no override of the slot.
#[test]
fn unassigned_slot_keeps_the_mains_default_content() {
    let doc = slots_document_with(false);
    let (bytes, warnings) = export(&doc, None).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let archive = Archive::open(&bytes).unwrap();
    let (schema, message) = decode_canvas(&archive.canvas).unwrap();
    let msg = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
    let changes = msg.get("nodeChanges").unwrap().list();
    let inst = changes.iter().find(|n| n.get("type").and_then(Kv::str) == Some("INSTANCE")).unwrap();
    assert!(
        !inst.get("componentPropAssignments").map(Kv::list).unwrap_or_default().iter().any(|a| a
            .get("varValue")
            .and_then(|v| v.get("dataType"))
            .and_then(Kv::str)
            == Some("SLOT_CONTENT_ID"))
    );
    assert!(!changes.iter().any(|n| n.get("isSlotContent").and_then(Kv::bool) == Some(true)));

    let (back, _) = import(&bytes, "Slots").unwrap();
    let slot = instance_slot(&back);
    assert!(!slot.overrides.contains("slot"));
    assert_eq!(names(&back, slot.id), ["Default"]);
}

/// Variants share their set's slot property: each variant's layer names its own copy of the
/// definition, and an instance of one variant still gets its own slot content.
#[test]
fn a_variants_slot_property_and_content_round_trip() {
    let doc = set_slots_document();
    let (bytes, warnings) = export(&doc, None).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let (back, _) = import(&bytes, "Set slots").unwrap();
    for name in ["Size=Small", "Size=Large"] {
        let v = back.nodes().find(|n| n.name == name).unwrap();
        let NodeKind::Component { props, .. } = &v.kind else { panic!("{name} is a component") };
        assert!(matches!(props.get("Content"), Some(PropDef::Slot { .. })), "{name}: {props:?}");
        let slot = v.children.iter().filter_map(|c| back.node(*c)).find(|n| n.prop_refs.contains_key("slot")).unwrap();
        assert_eq!(slot.prop_refs.get("slot").map(String::as_str), Some("Content"), "{name}");
    }
    let inst = back.nodes().find(|n| matches!(n.kind, NodeKind::Instance { .. })).unwrap();
    let slot = inst.children.iter().filter_map(|c| back.node(*c)).find(|n| n.overrides.contains("slot")).unwrap();
    assert_eq!(names(&back, slot.id), ["Mine"]);
}

/// Preferred slot components have no public field in the format, so they are dropped — but not
/// silently: the exporter warns, the way it does for an instance swap's preferred values.
#[test]
fn preferred_slot_components_are_left_out_with_a_warning() {
    const WARN: &str = "preferred components of slot properties were left out";
    let plain = slots_document();
    let (_, warnings) = export(&plain, None).unwrap();
    assert!(!warnings.iter().any(|w| w == WARN), "no preferred components: {warnings:?}");

    let mut doc = slots_document();
    let main = doc.nodes().find(|n| matches!(n.kind, NodeKind::Component { .. })).unwrap().id;
    let preferred = doc.alloc_id();
    let NodeKind::Component { props, .. } = &mut doc.node_mut(main).unwrap().kind else { panic!("not a component") };
    props.insert("Content".to_string(), PropDef::Slot { preferred: vec![preferred] });
    let (_, warnings) = export(&doc, None).unwrap();
    assert!(warnings.iter().any(|w| w == WARN), "preferred components were dropped without a warning: {warnings:?}");
}
