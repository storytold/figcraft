//! `SLOT` layers of the REST file JSON: bound to their slot property in a main component, and
//! holding each instance's own content (synthetic files).

use super::*;
use serde_json::json;

fn at(x: f64, y: f64) -> Value {
    json!([[1, 0, x], [0, 1, y]])
}

pub(crate) fn file(children: Value) -> Value {
    json!({"name": "Slots", "schemaVersion": 0, "document": {"id": "0:0", "name": "Document", "type": "DOCUMENT", "children": [
        {"id": "0:1", "name": "Page", "type": "CANVAS", "children": children}
    ]}})
}

fn content_id(local: u32) -> Value {
    json!({"guid": {"sessionID": 5, "localID": local}})
}

fn slot(id: &str, name: &str, children: Value) -> Value {
    json!({"id": id, "name": name, "type": "SLOT", "relativeTransform": at(10.0, 40.0), "size": {"x": 280, "y": 100}, "children": children})
}

fn label(id: &str, text: &str) -> Value {
    json!({"id": id, "name": text, "type": "TEXT", "characters": text, "relativeTransform": at(0.0, 0.0), "size": {"x": 100, "y": 20}})
}

/// A "Chip" component (`1:9`), a "Container" component (`1:2`) with a "Title" text and an empty
/// "Slot" (property `Slot#1:3`, default content 5:1), and the given layers after them.
pub(crate) fn container_file(more: Vec<Value>) -> Value {
    let mut layers = vec![
        json!({"id": "1:9", "name": "Chip", "type": "COMPONENT", "relativeTransform": at(0.0, 400.0), "size": {"x": 40, "y": 20}, "children": [
            {"id": "1:10", "name": "Dot", "type": "ELLIPSE", "relativeTransform": at(2.0, 2.0), "size": {"x": 16, "y": 16}}
        ]}),
        json!({"id": "1:2", "name": "Container", "type": "COMPONENT", "relativeTransform": at(0.0, 0.0), "size": {"x": 300, "y": 160},
               "componentPropertyDefinitions": {"Slot#1:3": {"type": "SLOT", "defaultValue": content_id(1)}},
               "children": [label("1:4", "Title"), slot("1:5", "Slot", json!([]))]}),
    ];
    layers.extend(more);
    file(Value::Array(layers))
}

/// An instance `id` of Container at `x` whose slot holds `content` (property value `value`).
pub(crate) fn container(id: &str, x: f64, value: Value, content: Value) -> Value {
    json!({"id": id, "name": "Container", "type": "INSTANCE", "componentId": "1:2", "relativeTransform": at(x, 0.0), "size": {"x": 300, "y": 160},
           "componentProperties": {"Slot#1:3": {"type": "SLOT", "value": value}},
           "children": [label(&format!("I{id};1:4"), "Title"), slot(&format!("I{id};1:5"), "Slot", content)]})
}

pub(crate) fn chip(id: &str) -> Value {
    json!({"id": id, "name": "Chip", "type": "INSTANCE", "componentId": "1:9", "relativeTransform": at(0.0, 30.0), "size": {"x": 40, "y": 20}, "children": [
        {"id": format!("I{id};1:10"), "name": "Dot", "type": "ELLIPSE", "relativeTransform": at(2.0, 2.0), "size": {"x": 16, "y": 16}}
    ]})
}

fn by_rest<'d>(doc: &'d Document, v: &Value, rest: &str) -> &'d Node {
    // Names repeat across copies: find by position in the file's tree instead.
    fn path(v: &Value, rest: &str, out: &mut Vec<usize>) -> bool {
        if v.get("id").and_then(Value::as_str) == Some(rest) {
            return true;
        }
        for (i, c) in v.get("children").and_then(Value::as_array).into_iter().flatten().enumerate() {
            out.push(i);
            if path(c, rest, out) {
                return true;
            }
            out.pop();
        }
        false
    }
    let mut p = Vec::new();
    assert!(path(&v["document"]["children"][0], rest, &mut p), "no {rest} in the file");
    let mut id = doc.pages[0];
    for i in p {
        id = doc.node(id).unwrap().children[i];
    }
    doc.node(id).unwrap()
}

fn names(doc: &Document, n: &Node) -> Vec<String> {
    n.children.iter().map(|c| doc.node(*c).unwrap().name.clone()).collect()
}

#[test]
fn slot_layers_bind_to_their_property_and_keep_instance_content() {
    let v = container_file(vec![
        container("2:1", 400.0, content_id(7), json!([label("2:2", "Reports in progress"), chip("2:3")])),
        container("3:1", 800.0, content_id(1), json!([])),
    ]);
    let (doc, warnings) = import(&v, &no_images).unwrap();
    assert!(warnings.iter().all(|w| !w.contains("SLOT") && !w.contains("slot")), "{warnings:?}");

    let main = by_rest(&doc, &v, "1:2");
    let NodeKind::Component { props, .. } = &main.kind else { panic!("{:?}", main.kind) };
    assert_eq!(props.get("Slot"), Some(&PropDef::Slot { preferred: Vec::new() }), "named without its #id suffix");
    let main_slot = by_rest(&doc, &v, "1:5");
    assert!(matches!(main_slot.kind, NodeKind::Frame));
    assert_eq!(main_slot.prop_refs.get("slot").map(String::as_str), Some("Slot"));
    assert!(main_slot.children.is_empty(), "the main's slot stays empty");

    let filled = by_rest(&doc, &v, "I2:1;1:5");
    assert!(filled.overrides.contains("slot"), "the instance's own content");
    assert!(filled.prop_refs.is_empty(), "an instance's slot follows its main's");
    assert_eq!(names(&doc, filled), ["Reports in progress", "Chip"]);
    let chip = by_rest(&doc, &v, "2:3");
    assert!(matches!(chip.kind, NodeKind::Instance { .. }), "instances in slot content link to their own main");

    let default = by_rest(&doc, &v, "I3:1;1:5");
    assert!(!default.overrides.contains("slot"), "the main's default content follows the main");
}

#[test]
fn slot_ownership_without_values_follows_the_content() {
    let mut owned = container("2:1", 400.0, json!(null), json!([label("2:2", "Own")]));
    owned["componentProperties"] = json!({});
    let mut empty = container("3:1", 800.0, json!(null), json!([]));
    empty["componentProperties"] = json!({});
    let v = container_file(vec![owned, empty]);
    let (doc, _) = import(&v, &no_images).unwrap();
    assert!(by_rest(&doc, &v, "I2:1;1:5").overrides.contains("slot"));
    assert!(!by_rest(&doc, &v, "I3:1;1:5").overrides.contains("slot"));
}

#[test]
fn slots_match_by_reference_then_name_then_the_one_left() {
    let comp = json!({"id": "1:2", "name": "Card", "type": "COMPONENT", "relativeTransform": at(0.0, 0.0), "size": {"x": 300, "y": 300},
    "componentPropertyDefinitions": {
        "Header#1:3": {"type": "SLOT", "defaultValue": content_id(1)},
        "Body#1:4": {"type": "SLOT", "defaultValue": content_id(2)},
        "Footer#1:5": {"type": "SLOT", "defaultValue": content_id(3)},
        "Visible#1:6": {"type": "BOOLEAN", "defaultValue": true}
    },
    "children": [
        {"id": "1:7", "name": "Top", "type": "SLOT", "componentPropertyReferences": {"visible": "Visible#1:6", "slot": "Header#1:3"},
         "relativeTransform": at(0.0, 0.0), "size": {"x": 300, "y": 50}},
        {"id": "1:8", "name": "Body", "type": "SLOT", "relativeTransform": at(0.0, 50.0), "size": {"x": 300, "y": 50}},
        {"id": "1:9", "name": "Bottom", "type": "SLOT", "relativeTransform": at(0.0, 100.0), "size": {"x": 300, "y": 50}}
    ]});
    let v = file(json!([comp]));
    let (doc, warnings) = import(&v, &no_images).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let bound = |rest: &str| by_rest(&doc, &v, rest).prop_refs.get("slot").cloned();
    assert_eq!(bound("1:7").as_deref(), Some("Header"));
    assert_eq!(by_rest(&doc, &v, "1:7").prop_refs.get("visible").map(String::as_str), Some("Visible"));
    assert_eq!(bound("1:8").as_deref(), Some("Body"));
    assert_eq!(bound("1:9").as_deref(), Some("Footer"));
}

#[test]
fn variants_share_their_set_slot_property() {
    let variant = |id: &str, name: &str, x: f64| {
        json!({"id": id, "name": name, "type": "COMPONENT", "relativeTransform": at(x, 0.0), "size": {"x": 100, "y": 100},
               "children": [slot(&format!("{id}s"), "Content", json!([]))]})
    };
    let set = json!({"id": "1:1", "name": "Panel", "type": "COMPONENT_SET", "relativeTransform": at(0.0, 0.0), "size": {"x": 300, "y": 100},
        "componentPropertyDefinitions": {"Size": {"type": "VARIANT", "defaultValue": "Small"}, "Content#2:1": {"type": "SLOT", "defaultValue": content_id(1)}},
        "children": [variant("1:2", "Size=Small", 0.0), variant("1:3", "Size=Large", 150.0)]});
    let inst = json!({"id": "4:1", "name": "Panel", "type": "INSTANCE", "componentId": "1:3", "relativeTransform": at(0.0, 300.0), "size": {"x": 100, "y": 100},
        "componentProperties": {"Content#2:1": {"type": "SLOT", "value": content_id(9)}},
        "children": [slot("I4:1;1:3s", "Content", json!([label("4:2", "Mine")]))]});
    let v = file(json!([set, inst]));
    let (doc, warnings) = import(&v, &no_images).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    for rest in ["1:2s", "1:3s"] {
        assert_eq!(by_rest(&doc, &v, rest).prop_refs.get("slot").map(String::as_str), Some("Content"), "{rest}");
    }
    assert!(by_rest(&doc, &v, "I4:1;1:3s").overrides.contains("slot"));
}

#[test]
fn nested_instances_own_the_slots_of_their_main() {
    // A "Screen" main places a Container whose slot holds a chip; an instance of Screen places
    // other content in that nested slot.
    let screen = json!({"id": "6:1", "name": "Screen", "type": "COMPONENT", "relativeTransform": at(0.0, 600.0), "size": {"x": 300, "y": 160},
        "children": [container("6:2", 0.0, content_id(7), json!([chip("6:3")]))]});
    let nested = container("I7:1;6:2", 0.0, content_id(8), json!([label("7:2", "Other")]));
    let outer = json!({"id": "7:1", "name": "Screen", "type": "INSTANCE", "componentId": "6:1", "relativeTransform": at(400.0, 600.0), "size": {"x": 300, "y": 160},
        "children": [nested]});
    let v = container_file(vec![screen, outer]);
    let (doc, _) = import(&v, &no_images).unwrap();
    let in_main = by_rest(&doc, &v, "I6:2;1:5");
    assert!(in_main.overrides.contains("slot") && in_main.prop_refs.is_empty());
    assert!(matches!(by_rest(&doc, &v, "6:3").kind, NodeKind::Instance { .. }));
    let in_outer = by_rest(&doc, &v, "II7:1;6:2;1:5");
    assert!(in_outer.overrides.contains("slot"));
    assert_eq!(names(&doc, in_outer), ["Other"]);
}

#[test]
fn slots_outside_components_or_without_a_property_stay_frames() {
    let mut no_def = container_file(Vec::new());
    no_def["document"]["children"][0]["children"][1].as_object_mut().unwrap().remove("componentPropertyDefinitions");
    let loose = slot("8:1", "Loose", json!([label("8:2", "Kept")]));
    no_def["document"]["children"][0]["children"].as_array_mut().unwrap().push(loose);
    let (doc, warnings) = import(&no_def, &no_images).unwrap();
    assert!(warnings.iter().any(|w| w.starts_with("slot layers without a matching slot property were imported as frames (×2)")), "{warnings:?}");
    let kept = by_rest(&doc, &no_def, "8:1");
    assert!(matches!(kept.kind, NodeKind::Frame) && kept.prop_refs.is_empty());
    assert_eq!(names(&doc, kept), ["Kept"]);
    assert!(by_rest(&doc, &no_def, "1:5").prop_refs.is_empty());
}

#[test]
fn slot_settings_the_editor_cannot_keep_are_noted() {
    let mut v = container_file(Vec::new());
    v["document"]["children"][0]["children"][1]["componentPropertyDefinitions"]["Slot#1:3"]["slotSettings"] =
        json!({"stretchChildOnInsert": true, "displayEmptyByDefault": true, "minChildren": null, "maxChildren": 3});
    v["document"]["children"][0]["children"][1]["componentPropertyDefinitions"]["Slot#1:3"]["preferredValues"] =
        json!([{"type": "COMPONENT", "key": "abc"}]);
    let (_, warnings) = import(&v, &no_images).unwrap();
    assert!(warnings.iter().any(|w| w == "non-default slot settings are not represented and were skipped"), "{warnings:?}");
    assert!(warnings.iter().any(|w| w == "preferred slot component keys cannot be resolved to file nodes and were skipped"), "{warnings:?}");
    let mut plain = container_file(Vec::new());
    plain["document"]["children"][0]["children"][1]["componentPropertyDefinitions"]["Slot#1:3"]["slotSettings"] =
        json!({"stretchChildOnInsert": true, "minChildren": null, "maxChildren": null, "allowPreferredValuesOnly": false});
    assert!(import(&plain, &no_images).unwrap().1.is_empty());
}

#[test]
fn junk_slot_json_never_panics() {
    for junk in [json!(null), json!(1), json!("x"), json!([]), json!({"guid": null}), json!({"type": "SLOT"})] {
        let mut v = container_file(vec![container("2:1", 400.0, junk.clone(), json!([label("2:2", "A")]))]);
        v["document"]["children"][0]["children"][1]["componentPropertyDefinitions"]["Slot#1:3"] =
            json!({"type": "SLOT", "defaultValue": junk.clone(), "slotSettings": junk.clone(), "preferredValues": junk.clone()});
        v["document"]["children"][0]["children"][1]["children"][1]["componentPropertyReferences"] = json!({"slot": junk});
        assert!(import(&v, &no_images).is_ok());
    }
}
