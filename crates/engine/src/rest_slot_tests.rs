//! Slots opened from REST file JSON: instances keep their own slot content through edits and
//! moves of the mains, and through saving.

use crate::Session;
use crate::components::tests::{ex, healthy, idv};
use crate::components::*;
use base64::Engine as _;
use figcraft_doc::*;
use serde_json::{Value, json};

fn at(x: f64, y: f64) -> Value {
    json!([[1, 0, x], [0, 1, y]])
}

fn text(id: &str, s: &str) -> Value {
    json!({"id": id, "name": s, "type": "TEXT", "characters": s, "relativeTransform": at(0.0, 0.0), "size": {"x": 100, "y": 20}})
}

/// A Chip main, a Container main with an empty "Slot", and an instance of Container whose slot
/// holds a text and a chip.
fn file() -> Value {
    let slot = |id: &str, kids: Value| json!({"id": id, "name": "Slot", "type": "SLOT", "relativeTransform": at(10.0, 40.0), "size": {"x": 280, "y": 100}, "children": kids});
    let guid = |l: u32| json!({"guid": {"sessionID": 5, "localID": l}});
    let chip = json!({"id": "2:3", "name": "Chip", "type": "INSTANCE", "componentId": "1:9", "relativeTransform": at(0.0, 30.0), "size": {"x": 40, "y": 20},
        "children": [{"id": "I2:3;1:10", "name": "Dot", "type": "ELLIPSE", "relativeTransform": at(2.0, 2.0), "size": {"x": 16, "y": 16}}]});
    json!({"name": "Slots", "document": {"id": "0:0", "type": "DOCUMENT", "children": [{"id": "0:1", "name": "Page", "type": "CANVAS", "children": [
        {"id": "1:9", "name": "Chip", "type": "COMPONENT", "relativeTransform": at(0.0, 400.0), "size": {"x": 40, "y": 20}, "children": [
            {"id": "1:10", "name": "Dot", "type": "ELLIPSE", "relativeTransform": at(2.0, 2.0), "size": {"x": 16, "y": 16}}
        ]},
        {"id": "1:2", "name": "Container", "type": "COMPONENT", "relativeTransform": at(0.0, 0.0), "size": {"x": 300, "y": 160},
         "componentPropertyDefinitions": {"Slot#1:3": {"type": "SLOT", "defaultValue": guid(1)}},
         "children": [text("1:4", "Title"), slot("1:5", json!([]))]},
        {"id": "2:1", "name": "Container", "type": "INSTANCE", "componentId": "1:2", "relativeTransform": at(400.0, 0.0), "size": {"x": 300, "y": 160},
         "componentProperties": {"Slot#1:3": {"type": "SLOT", "value": guid(7)}},
         "children": [text("I2:1;1:4", "Title"), slot("I2:1;1:5", json!([text("2:2", "Reports in progress"), chip]))]}
    ]}]}})
}

fn named(doc: &Document, kind: fn(&NodeKind) -> bool, name: &str) -> NodeId {
    doc.nodes().find(|n| n.name == name && kind(&n.kind)).map(|n| n.id).unwrap_or_else(|| panic!("no {name}"))
}

/// The instance's slot and the names of what it holds.
fn slot_content(s: &Session) -> (NodeId, Vec<String>) {
    let doc = s.doc().unwrap();
    let inst = named(doc, |k| matches!(k, NodeKind::Instance { .. }), "Container");
    let (slot, _) = slots_of(doc, inst).into_iter().next().expect("the instance has its slot");
    let names = doc.node(slot).unwrap().children.iter().map(|c| doc.node(*c).unwrap().name.clone()).collect();
    (slot, names)
}

#[test]
fn rest_slot_content_survives_main_edits_moves_and_saving() {
    let mut s = Session::new();
    let data = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&file()).unwrap());
    ex(&mut s, "file.open", json!({"data": data, "name": "slots.json"}));
    healthy(&s);
    let want = vec!["Reports in progress".to_string(), "Chip".to_string()];
    let (slot, names) = slot_content(&s);
    assert_eq!(names, want);
    let doc = s.doc().unwrap();
    assert_eq!(slot_name(doc, slot).as_deref(), Some("Slot"));
    assert!(slot_edited(doc, slot), "the instance's own content");
    let chip = named(doc, |k| matches!(k, NodeKind::Instance { .. }), "Chip");
    assert!(!inside_instance(doc, chip), "content stands alone");

    // Edit and move both mains (the container's and the one used in the content).
    let main = named(doc, |k| matches!(k, NodeKind::Component { .. }), "Container");
    let chip_main = named(doc, |k| matches!(k, NodeKind::Component { .. }), "Chip");
    let title = doc.node(main).unwrap().children[0];
    ex(&mut s, "node.set", json!({"id": title, "props": {"fill": "#ff0000"}}));
    ex(&mut s, "node.set", json!({"id": main, "props": {"width": 320}}));
    let holder = idv(&ex(&mut s, "frame.create", json!({"x": -1000, "y": 0, "width": 900, "height": 900, "name": "Design system"})));
    ex(&mut s, "node.reparent", json!({"ids": [main, chip_main], "parent": holder}));
    healthy(&s);
    assert_eq!(slot_content(&s).1, want, "kept across syncs");

    // Saving and opening again keeps it.
    let bytes = figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mut again = Session::new();
    ex(&mut again, "file.open", json!({"data": data, "name": "slots.figcraft"}));
    healthy(&again);
    let (slot, names) = slot_content(&again);
    assert_eq!(names, want);
    assert!(slot_edited(again.doc().unwrap(), slot));

    // Resetting the slot shows the main's (empty) default content.
    ex(&mut again, "instance.resetSlot", json!({"id": slot}));
    assert!(slot_content(&again).1.is_empty());
}
