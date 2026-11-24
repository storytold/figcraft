//! A variant property bound to a string variable (REST `boundVariables.componentProperties`, the
//! form a design kit's Light/Dark `Mode` takes) picks the variant the variable names in the
//! instance's mode as soon as the file opens: an instance in a frame set to Dark shows the Dark
//! variant, one in a Light frame the Light one.

use base64::Engine as _;
use serde_json::{Value, json};

use crate::Session;
use figcraft_doc::{NodeId, NodeKind};

fn at(x: f64, y: f64) -> Value {
    json!([[1.0, 0.0, x], [0.0, 1.0, y]])
}

fn solid(v: f64) -> Value {
    json!({"type": "SOLID", "color": {"r": v, "g": v, "b": v, "a": 1.0}})
}

/// The local variables: a `Scheme` collection with Light and Dark modes and a string variable
/// `Mode` that holds each mode's name.
fn variables() -> Value {
    json!({"meta": {
        "variableCollections": {"VC:1": {"id": "VC:1", "name": "Scheme", "modes": [{"modeId": "1:0", "name": "Light"}, {"modeId": "1:1", "name": "Dark"}],
                                         "defaultModeId": "1:0", "variableIds": ["VariableID:1"]}},
        "variables": {"VariableID:1": {"id": "VariableID:1", "name": "Mode", "variableCollectionId": "VC:1", "resolvedType": "STRING",
                                       "valuesByMode": {"1:0": "Light", "1:1": "Dark"}}}
    }})
}

/// An instance of the Light variant called `name` at `x`, its `Mode` bound to the variable.
fn instance(id: &str, name: &str) -> Value {
    json!({"id": id, "name": name, "type": "INSTANCE", "componentId": "1:2", "relativeTransform": at(10.0, 10.0), "size": {"x": 40, "y": 20},
           "componentProperties": {"Mode": {"type": "VARIANT", "value": "Light"}},
           "boundVariables": {"componentProperties": {"Mode": {"type": "VARIABLE_ALIAS", "id": "VariableID:1"}}},
           "fills": [solid(1.0)]})
}

fn frame(id: &str, name: &str, x: f64, mode: &str, child: Value) -> Value {
    json!({"id": id, "name": name, "type": "FRAME", "relativeTransform": at(x, 200.0), "size": {"x": 60, "y": 40},
           "explicitVariableModes": {"VC:1": mode}, "children": [child]})
}

fn file() -> Value {
    json!({
        "name": "Modes", "schemaVersion": 0,
        "document": {"id": "0:0", "type": "DOCUMENT", "children": [{"id": "0:1", "name": "Page", "type": "CANVAS", "children": [
            {"id": "1:1", "name": "Chip", "type": "COMPONENT_SET", "relativeTransform": at(0.0, 0.0), "size": {"x": 120, "y": 40},
             "componentPropertyDefinitions": {"Mode": {"type": "VARIANT", "defaultValue": "Light", "variantOptions": ["Light", "Dark"]}},
             "children": [
                {"id": "1:2", "name": "Mode=Light", "type": "COMPONENT", "relativeTransform": at(0.0, 0.0), "size": {"x": 40, "y": 20}, "fills": [solid(1.0)]},
                {"id": "1:3", "name": "Mode=Dark", "type": "COMPONENT", "relativeTransform": at(60.0, 0.0), "size": {"x": 40, "y": 20}, "fills": [solid(0.0)]}
             ]},
            frame("2:1", "Light screen", 0.0, "1:0", instance("3:1", "In light")),
            frame("2:2", "Dark screen", 100.0, "1:1", instance("3:2", "In dark"))
        ]}]},
        "components": {"1:2": {"key": "k2", "name": "Mode=Light", "componentSetId": "1:1"}, "1:3": {"key": "k3", "name": "Mode=Dark", "componentSetId": "1:1"}},
        "componentSets": {"1:1": {"key": "s1", "name": "Chip"}},
        "styles": {}
    })
}

/// A rectangle called `name` filled `fill` (grey level).
fn rect(id: &str, name: &str, fill: f64) -> Value {
    json!({"id": id, "name": name, "type": "RECTANGLE", "relativeTransform": at(0.0, 0.0), "size": {"x": 40, "y": 20}, "fills": [solid(fill)]})
}

/// A kit: a Light/Dark `Chip` set whose variants hold a `Base` (light or dark grey), a `Tint`
/// (white or black) and a `Label` text reading property `Label#9:9`, which the set lists under
/// another id (as an imported library's can); a `Button` main holding a Chip whose `Tint` it
/// makes red (an override of the main's), the Chip's `Mode` bound to the variable; and a Button
/// instance in a Dark frame that sets the label to "Stop" (`expanded`: the file lists the
/// instance's layers, as a file read through the API does).
fn kit_file(expanded: bool) -> Value {
    let label = |id: &str| {
        json!({"id": id, "name": "Label", "type": "TEXT", "relativeTransform": at(0.0, 0.0), "size": {"x": 30, "y": 16},
               "characters": "Label", "style": {"fontSize": 12}, "componentPropertyReferences": {"characters": "Label#9:9"}})
    };
    let variant = |id: &str, mode: &str, base: f64, tint: f64, ids: [&str; 3]| {
        json!({"id": id, "name": format!("Mode={mode}"), "type": "COMPONENT", "relativeTransform": at(0.0, 0.0), "size": {"x": 40, "y": 20},
               "children": [rect(ids[0], "Base", base), rect(ids[1], "Tint", tint), label(ids[2])]})
    };
    let chip = |id: &str, tint: f64, inner: [&str; 3], label_text: &str| {
        let mut t = label(inner[2]);
        t["characters"] = json!(label_text);
        json!({"id": id, "name": "Chip", "type": "INSTANCE", "componentId": "1:2", "relativeTransform": at(0.0, 0.0), "size": {"x": 40, "y": 20},
               "componentProperties": {"Mode": {"type": "VARIANT", "value": "Light"}, "Label#9:9": {"type": "TEXT", "value": label_text}},
               "boundVariables": {"componentProperties": {"Mode": {"type": "VARIABLE_ALIAS", "id": "VariableID:1"}}},
               "children": [rect(inner[0], "Base", 0.8), rect(inner[1], "Tint", tint), t]})
    };
    let red = json!([{"type": "SOLID", "color": {"r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0}}]);
    let mut main_chip = chip("4:2", 1.0, ["I4:2;1:4", "I4:2;1:5", "I4:2;1:6"], "Label");
    main_chip["children"][1]["fills"] = red.clone();
    let mut used_chip = chip("I5:1;4:2", 1.0, ["I5:1;4:2;1:4", "I5:1;4:2;1:5", "I5:1;4:2;1:6"], "Stop");
    used_chip["children"][1]["fills"] = red;
    json!({
        "name": "Kit", "schemaVersion": 0,
        "document": {"id": "0:0", "type": "DOCUMENT", "children": [{"id": "0:1", "name": "Page", "type": "CANVAS", "children": [
            {"id": "1:1", "name": "Chip", "type": "COMPONENT_SET", "relativeTransform": at(0.0, 0.0), "size": {"x": 120, "y": 40},
             "componentPropertyDefinitions": {"Mode": {"type": "VARIANT", "defaultValue": "Light", "variantOptions": ["Light", "Dark"]},
                                              "Label#1:5": {"type": "TEXT", "defaultValue": "Label"}},
             "children": [variant("1:2", "Light", 0.8, 1.0, ["1:4", "1:5", "1:6"]), variant("1:3", "Dark", 0.2, 0.0, ["1:7", "1:8", "1:9"])]},
            {"id": "4:1", "name": "Button", "type": "COMPONENT", "relativeTransform": at(200.0, 0.0), "size": {"x": 40, "y": 20}, "children": [main_chip]},
            {"id": "2:2", "name": "Dark screen", "type": "FRAME", "relativeTransform": at(0.0, 200.0), "size": {"x": 60, "y": 40},
             "explicitVariableModes": {"VC:1": "1:1"},
             "children": [{"id": "5:1", "name": "Stop", "type": "INSTANCE", "componentId": "4:1", "relativeTransform": at(10.0, 10.0),
                           "size": {"x": 40, "y": 20}, "children": if expanded { json!([used_chip]) } else { json!([]) }}]}
        ]}]},
        "components": {"1:2": {"key": "k2", "name": "Mode=Light", "componentSetId": "1:1"}, "1:3": {"key": "k3", "name": "Mode=Dark", "componentSetId": "1:1"},
                       "4:1": {"key": "k4", "name": "Button"}},
        "componentSets": {"1:1": {"key": "s1", "name": "Chip"}},
        "styles": {}
    })
}

/// The name of the variant the instance called `name` shows.
fn variant_of(s: &Session, name: &str) -> String {
    let doc = s.doc().unwrap();
    let n = doc.nodes().find(|n| n.name == name).unwrap();
    let NodeKind::Instance { component, .. } = &n.kind else { panic!("{name} is not an instance") };
    doc.node(*component).unwrap().name.clone()
}

#[test]
fn a_variant_bound_to_a_mode_variable_follows_the_frames_mode_on_open() {
    let data = base64::engine::general_purpose::STANDARD.encode(file().to_string());
    let mut s = Session::new();
    let r = s.execute("file.open", &json!({"data": data, "name": "modes.json", "variables": variables()})).unwrap();
    assert_eq!(r["warnings"], json!([]), "{r}");
    assert_eq!(variant_of(&s, "In light"), "Mode=Light");
    assert_eq!(variant_of(&s, "In dark"), "Mode=Dark");
    // The binding stays, so a later change of the frame's mode switches the variant again.
    let doc = s.doc().unwrap();
    let dark = doc.nodes().find(|n| n.name == "In dark").unwrap();
    assert!(dark.bound.contains_key("componentProperties/Mode"), "{:?}", dark.bound);
}

/// The fill (grey level, red as 1.0 + 10) of the layer called `name` inside the layer called `inside`.
fn fill_in(s: &Session, inside: &str, name: &str) -> f32 {
    let doc = s.doc().unwrap();
    let outer =
        doc.nodes().find(|n| n.name == inside && doc.ancestors(n.id).iter().any(|a| doc.node(*a).is_some_and(|p| p.name == "Dark screen"))).unwrap();
    let id = doc.subtree(outer.id).into_iter().find(|i| doc.node(*i).is_some_and(|n| n.name == name)).unwrap();
    let c = doc.node(id).unwrap().fills[0].solid_color().unwrap();
    if c.r > 0.9 && c.g < 0.1 { 11.0 } else { c.r }
}

fn open_kit(expanded: bool) -> Session {
    let data = base64::engine::general_purpose::STANDARD.encode(kit_file(expanded).to_string());
    let mut s = Session::new();
    s.execute("file.open", &json!({"data": data, "name": "kit.json", "variables": variables()})).unwrap();
    s
}

/// The Chip instance inside the Dark frame.
fn dark_chip(s: &Session) -> NodeId {
    let doc = s.doc().unwrap();
    let in_dark = |n: &figcraft_doc::Node| doc.ancestors(n.id).iter().any(|a| doc.node(*a).is_some_and(|p| p.name == "Dark screen"));
    doc.nodes().find(|n| n.name == "Chip" && matches!(n.kind, NodeKind::Instance { .. }) && in_dark(n)).unwrap().id
}

#[test]
fn a_nested_instance_switching_variant_keeps_its_enclosing_mains_overrides() {
    {
        let expanded = true;
        let s = open_kit(expanded);
        let doc = s.doc().unwrap();
        let NodeKind::Instance { component, .. } = &doc.node(dark_chip(&s)).unwrap().kind else { panic!("not an instance") };
        assert_eq!(doc.node(*component).unwrap().name, "Mode=Dark", "expanded: {expanded}");
        // The Dark variant's own base, the enclosing Button main's red tint over it.
        assert!((fill_in(&s, "Stop", "Base") - 0.2).abs() < 0.01, "expanded: {expanded}: {}", fill_in(&s, "Stop", "Base"));
        assert_eq!(fill_in(&s, "Stop", "Tint"), 11.0, "expanded: {expanded}: the Button main's red tint stays");
    }
}

#[test]
fn a_text_reads_the_property_the_instance_sets_though_the_set_lists_it_under_another_id() {
    let s = open_kit(true);
    let doc = s.doc().unwrap();
    let label = doc.subtree(dark_chip(&s)).into_iter().filter_map(|i| doc.node(i)).find(|n| n.name == "Label").unwrap();
    let NodeKind::Text(t) = &label.kind else { panic!("not text") };
    assert_eq!(t.characters, "Stop");
}

#[test]
fn review_bound_text_refreshes_children_without_a_variant_switch_on_open() {
    let mut file = kit_file(true);
    // This case binds a defined property; the separate alias fixture above intentionally uses
    // different ids for an enclosing main's explicit override and must retain that override.
    let definitions = &mut file["document"]["children"][0]["children"][0]["componentPropertyDefinitions"];
    let definition = definitions.as_object_mut().unwrap().remove("Label#1:5").unwrap();
    definitions["Label#9:9"] = definition;
    let used = &mut file["document"]["children"][0]["children"][2]["children"][0]["children"][0];
    used["componentProperties"]["Label#9:9"]["value"] = json!("Label");
    used["children"][2]["characters"] = json!("Label");
    file["document"]["children"][0]["children"][1]["children"][0]["boundVariables"]["componentProperties"] =
        json!({"Label#9:9":{"type":"VARIABLE_ALIAS","id":"VariableID:1"}});
    file["document"]["children"][0]["children"][2]["children"][0]["children"][0]["boundVariables"]["componentProperties"] =
        json!({"Label#9:9":{"type":"VARIABLE_ALIAS","id":"VariableID:1"}});
    let data = base64::engine::general_purpose::STANDARD.encode(file.to_string());
    let mut s = Session::new();
    s.execute("file.open", &json!({"data":data,"name":"kit.json","variables":variables()})).unwrap();
    let doc = s.doc().unwrap();
    let chip = doc.node(dark_chip(&s)).unwrap();
    let NodeKind::Instance { component, .. } = &chip.kind else { panic!("not an instance") };
    assert_eq!(doc.node(*component).unwrap().name, "Mode=Light", "the component itself did not switch");
    let label = doc.subtree(chip.id).into_iter().filter_map(|id| doc.node(id)).find(|n| n.name == "Label").unwrap();
    let NodeKind::Text(t) = &label.kind else { panic!("not text") };
    assert_eq!(t.characters, "Dark", "the bound text must reach the rendered child; instance={:?}, binding={:?}", chip.kind, chip.bound);
}
