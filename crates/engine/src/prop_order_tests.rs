//! Component properties keep the order they were made in: renaming one never moves it (#1536),
//! and Make slot takes the name it is given (#1537).

use figcraft_doc::{NodeId, NodeKind};
use serde_json::{Value, json};

use crate::Session;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn idv(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// The property names `component.properties` lists for `id`, in order.
fn names(s: &mut Session, id: NodeId) -> Vec<String> {
    ex(s, "component.properties", json!({"id": id}))["properties"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect()
}

fn rename(s: &mut Session, id: NodeId, name: &str, new_name: &str) {
    ex(s, "component.editProperty", json!({"id": id, "name": name, "newName": new_name}));
}

/// A "Card" component with five frames made slots one by one ("Slot 1" … "Slot 5"). Returns
/// (card, frames).
fn card_with_slots(s: &mut Session) -> (NodeId, Vec<NodeId>) {
    let f = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 500, "height": 100, "name": "Card"})));
    let frames: Vec<NodeId> = (1..=5)
        .map(|k| {
            idv(&ex(s, "frame.create", json!({"x": (k - 1) * 100, "y": 0, "width": 90, "height": 90, "parent": f, "name": format!("Slot {k}")})))
        })
        .collect();
    let card = idv(&ex(s, "component.create", json!({"ids": [f]})));
    for fr in &frames {
        ex(s, "component.makeSlot", json!({"id": fr}));
    }
    (card, frames)
}

#[test]
fn renaming_a_slot_keeps_its_place() {
    let mut s = Session::new();
    let (card, _) = card_with_slots(&mut s);
    assert_eq!(names(&mut s, card), ["Slot 1", "Slot 2", "Slot 3", "Slot 4", "Slot 5"]);
    rename(&mut s, card, "Slot 1", "Icon 1");
    rename(&mut s, card, "Slot 2", "Icon 2");
    rename(&mut s, card, "Icon 1", "Icone 1");
    assert_eq!(names(&mut s, card), ["Icone 1", "Icon 2", "Slot 3", "Slot 4", "Slot 5"], "the issue's steps");
    // Renamed to sort first by name, the last one still stays last.
    rename(&mut s, card, "Slot 5", "A");
    assert_eq!(names(&mut s, card), ["Icone 1", "Icon 2", "Slot 3", "Slot 4", "A"]);
    // Undo puts the old name back in its place; a deleted property leaves the others in order.
    s.undo().unwrap();
    assert_eq!(names(&mut s, card), ["Icone 1", "Icon 2", "Slot 3", "Slot 4", "Slot 5"]);
    ex(&mut s, "component.deleteProperty", json!({"id": card, "name": "Icon 2"}));
    assert_eq!(names(&mut s, card), ["Icone 1", "Slot 3", "Slot 4", "Slot 5"]);
    // The order is saved with the file.
    let saved = serde_json::to_string(s.doc().unwrap()).unwrap();
    let back: figcraft_doc::Document = serde_json::from_str(&saved).unwrap();
    assert_eq!(back.node(card).map(|n| &n.kind), s.doc().unwrap().node(card).map(|n| &n.kind));
}

#[test]
fn every_kind_of_property_keeps_its_place_on_rename() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Size=Small"})));
    let label = idv(&ex(&mut s, "text.create", json!({"text": "Go", "x": 10, "y": 10, "parent": f})));
    let body = idv(&ex(&mut s, "frame.create", json!({"x": 100, "y": 0, "width": 90, "height": 90, "parent": f, "name": "Body"})));
    let icon_f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 300, "width": 20, "height": 20, "name": "Icon"})));
    let icon = idv(&ex(&mut s, "component.create", json!({"ids": [icon_f]})));
    let nested = idv(&ex(&mut s, "instance.create", json!({"component": icon, "x": 60, "y": 10, "parent": f})));
    let small = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    // Made in an order that isn't by name: Zoom (boolean), Label (text), Glyph (instance swap), Body (slot).
    ex(&mut s, "component.addProperty", json!({"id": small, "name": "Zoom", "type": "boolean"}));
    ex(&mut s, "component.addProperty", json!({"id": small, "name": "Label", "type": "text", "layer": label}));
    ex(&mut s, "component.addProperty", json!({"id": small, "name": "Glyph", "type": "instanceSwap", "layer": nested}));
    ex(&mut s, "component.makeSlot", json!({"id": body}));
    assert_eq!(names(&mut s, small), ["Zoom", "Label", "Glyph", "Body"]);
    for (old, new) in [("Zoom", "A zoom"), ("Label", "B label"), ("Glyph", "C glyph"), ("Body", "D body")] {
        rename(&mut s, small, old, new);
    }
    assert_eq!(names(&mut s, small), ["A zoom", "B label", "C glyph", "D body"]);
    rename(&mut s, small, "A zoom", "Zz");
    assert_eq!(names(&mut s, small), ["Zz", "B label", "C glyph", "D body"]);
    // In a set the shared properties keep their order on every variant, after the variant ones
    // (which keep theirs when renamed too).
    ex(&mut s, "component.addVariant", json!({"id": small, "property": "Size", "value": "Large"}));
    let set = s.doc().unwrap().node(small).and_then(|n| n.parent).unwrap();
    ex(&mut s, "component.addProperty", json!({"id": set, "name": "State", "type": "variant", "default": "Rest"}));
    assert_eq!(names(&mut s, set), ["Size", "State", "Zz", "B label", "C glyph", "D body"]);
    rename(&mut s, set, "Size", "Width");
    rename(&mut s, set, "C glyph", "Alpha");
    assert_eq!(names(&mut s, set), ["Width", "State", "Zz", "B label", "Alpha", "D body"]);
    let variants: Vec<NodeId> = s.doc().unwrap().node(set).unwrap().children.clone();
    for v in variants {
        assert_eq!(names(&mut s, v), ["Width", "State", "Zz", "B label", "Alpha", "D body"]);
    }
    // An instance lists them in the same order.
    let inst = idv(&ex(&mut s, "instance.create", json!({"component": set, "x": 0, "y": 600})));
    assert_eq!(names(&mut s, inst), ["Width", "State", "Zz", "B label", "Alpha", "D body"]);
}

#[test]
fn make_slot_takes_the_name_typed() {
    let mut s = Session::new();
    let (card, frames) = card_with_slots(&mut s);
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 40, "height": 40, "parent": card, "name": "Frame 12"})));
    let r = ex(&mut s, "component.makeSlot", json!({"id": f, "name": "  Avatar  "}));
    assert_eq!(r["name"], "Avatar");
    assert_eq!(crate::components::slot_name(s.doc().unwrap(), f).as_deref(), Some("Avatar"));
    assert_eq!(names(&mut s, card).last().map(String::as_str), Some("Avatar"), "a new slot goes last");
    // A name taken, or one a property can't have, is refused.
    for bad in ["Slot 1", "a=b", ""] {
        let other = frames[4];
        ex(&mut s, "component.removeSlot", json!({"id": other}));
        assert!(s.execute("component.makeSlot", &json!({"id": other, "name": bad})).is_err(), "{bad:?}");
        ex(&mut s, "component.makeSlot", json!({"id": other, "name": "Slot 5"}));
    }
    // Files from before the order was kept list their properties by name.
    let doc = s.doc_mut().unwrap();
    if let Some(NodeKind::Component { prop_order, .. }) = doc.node_mut(card).map(|n| &mut n.kind) {
        prop_order.clear();
    }
    let mut by_name = names(&mut s, card);
    let listed = by_name.clone();
    by_name.sort();
    assert_eq!(listed, by_name);
}
