//! A set's component properties belong to all its variants, as `component.addProperty` on a set
//! gives them: one made on a component before it was combined goes to every variant, so another
//! variant's layer can be bound to it and an instance switched to that variant keeps its value.

use serde_json::{Value, json};

use crate::Session;
use crate::components::tests::{ex, healthy, idv};
use figcraft_doc::NodeId;

/// A main component named `name` at `x`, made from a 120×40 frame holding a "Button" text: the
/// component and the text.
fn component(s: &mut Session, name: &str, x: f64) -> (NodeId, NodeId) {
    let f = idv(&ex(s, "frame.create", json!({"x": x, "y": 0, "width": 120, "height": 40, "name": name})));
    let t = idv(&ex(s, "text.create", json!({"text": "Button", "x": 8, "y": 8, "parent": f})));
    (idv(&ex(s, "component.create", json!({"ids": [f]}))), t)
}

/// `id`'s component property `name` as `component.properties` lists it.
fn property(s: &mut Session, id: NodeId, name: &str) -> Option<Value> {
    let all = ex(s, "component.properties", json!({"id": id}));
    all["properties"].as_array().and_then(|l| l.iter().find(|p| p["name"] == name).cloned())
}

#[test]
fn a_property_made_before_combining_goes_to_every_variant() {
    let mut s = Session::new();
    let (a, label) = component(&mut s, "State=Default", 0.0);
    let (b, b_label) = component(&mut s, "State=Hover", 200.0);
    ex(&mut s, "component.addProperty", json!({"id": a, "name": "Text", "type": "text", "layer": label}));
    let of_b = idv(&ex(&mut s, "instance.create", json!({"component": b, "x": 0, "y": 300})));
    ex(&mut s, "component.combineVariants", json!({"ids": [a, b]}));
    healthy(&s);
    let shared = property(&mut s, b, "Text").expect("the other variant has the property");
    assert_eq!((shared["type"].clone(), shared["default"].clone()), (json!("text"), json!("Button")));
    // The other variant's text can be bound to it, and its instance then shows the default.
    ex(&mut s, "component.bindProperty", json!({"layer": b_label, "property": "Text"}));
    assert_eq!(property(&mut s, of_b, "Text").map(|p| p["value"].clone()), Some(json!("Button")));
    // An instance switched to the other variant keeps the value it was given.
    let of_a = idv(&ex(&mut s, "instance.create", json!({"component": a, "x": 200, "y": 300})));
    ex(&mut s, "instance.setProperties", json!({"ids": [of_a], "properties": {"Text": "Buy"}}));
    ex(&mut s, "instance.setProperties", json!({"ids": [of_a], "properties": {"State": "Hover"}}));
    assert_eq!(property(&mut s, of_a, "Text").map(|p| p["value"].clone()), Some(json!("Buy")));
    healthy(&s);
}

#[test]
fn a_variant_keeps_its_own_property_of_the_same_name() {
    let mut s = Session::new();
    let (a, _) = component(&mut s, "State=Default", 0.0);
    let (b, _) = component(&mut s, "State=Hover", 200.0);
    ex(&mut s, "component.addProperty", json!({"id": a, "name": "Icon", "type": "text", "default": "Star"}));
    ex(&mut s, "component.addProperty", json!({"id": b, "name": "Icon", "type": "boolean", "default": false}));
    ex(&mut s, "component.combineVariants", json!({"ids": [a, b]}));
    healthy(&s);
    assert_eq!(property(&mut s, a, "Icon").map(|p| p["type"].clone()), Some(json!("text")));
    assert_eq!(property(&mut s, b, "Icon").map(|p| p["type"].clone()), Some(json!("boolean")));
}
