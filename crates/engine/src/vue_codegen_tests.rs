//! Vue code: `inspect.code {language: "vue"}` writes a single-file component — `<template>`, a
//! `<script setup>` with typed props, and a `<style scoped>` — for a layer tree, and a component
//! with its properties as props, its variants as `data-*` attributes and its slots as
//! `<slot>` elements. A Vue template holds no arbitrary expressions, so what React writes as a
//! braced expression (an interpolation, a `v-if`, a `:prop` binding) has its own spelling here.

use crate::Session;
use crate::codegen::CodeLang;
use crate::components::tests::{ex, idv};
use figcraft_doc::*;
use serde_json::json;

fn code(s: &mut Session, id: NodeId, lang: &str) -> String {
    ex(s, "inspect.code", json!({"id": id, "language": lang}))["code"].as_str().unwrap_or_default().to_string()
}

fn vue(s: &mut Session, id: NodeId) -> String {
    code(s, id, "vue")
}

fn component_code(s: &mut Session, id: NodeId, lang: &str) -> serde_json::Value {
    ex(s, "inspect.componentCode", json!({"id": id, "language": lang}))
}

/// A "Card" component with a "Title" text (text property) and a "Dot" ellipse (boolean
/// property "Show dot"). Returns (card, title, dot).
fn card(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let f = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Card"})));
    ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 100, "parent": f, "props": {"name": "Bg", "fill": "#eeeeee"}}));
    let title = idv(&ex(s, "text.create", json!({"text": "Hello", "x": 10, "y": 10, "parent": f, "props": {"name": "Title"}})));
    let dot = idv(&ex(s, "shape.ellipse", json!({"x": 170, "y": 10, "width": 20, "height": 20, "parent": f, "props": {"name": "Dot"}})));
    let c = idv(&ex(s, "component.create", json!({"ids": [f]})));
    ex(s, "component.addProperty", json!({"id": dot, "name": "Show dot", "type": "boolean", "default": true}));
    ex(s, "component.addProperty", json!({"id": c, "name": "Title", "type": "text", "layer": title}));
    (c, title, dot)
}

#[test]
fn a_layer_tree_becomes_a_single_file_component() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 200, "name": "Panel"})));
    ex(&mut s, "text.create", json!({"text": "Hi", "x": 10, "y": 10, "parent": f, "props": {"name": "Label"}}));
    let out = vue(&mut s, f);
    // The three blocks of an SFC, in order, with the tree in the template and the CSS scoped.
    assert!(out.starts_with("<template>\n"), "{out}");
    assert!(out.contains("</template>\n\n<style scoped>\n"), "{out}");
    assert!(out.ends_with("</style>\n"), "{out}");
    assert!(out.contains("<div class=\"panel\">"), "{out}");
    assert!(out.contains(">Hi</p>"), "{out}");
    // A tree without properties needs no script block.
    assert!(!out.contains("<script"), "{out}");
}

#[test]
fn components_take_their_properties_as_props() {
    let mut s = Session::new();
    let (c, _, _) = card(&mut s);
    let out = vue(&mut s, c);
    // Typed props with their defaults, and the template below.
    assert!(out.starts_with("<script setup lang=\"ts\">\n"), "{out}");
    assert!(out.contains("const props = withDefaults(\n  defineProps<{\n"), "{out}");
    assert!(out.contains("showDot?: boolean;"), "{out}");
    assert!(out.contains("title?: string;"), "{out}");
    assert!(out.contains("showDot: true,"), "{out}");
    assert!(out.contains("title: \"Hello\","), "{out}");
    assert!(out.contains("</script>\n\n<template>\n"), "{out}");
    // A text property is an interpolation, not React's braced expression.
    assert!(out.contains("{{ title }}"), "{out}");
}

#[test]
fn a_boolean_property_guards_its_layer_with_v_if() {
    let mut s = Session::new();
    let (c, _, _) = card(&mut s);
    let out = vue(&mut s, c);
    // Vue can't wrap an element in an expression: the guard rides on the element itself.
    assert!(out.contains("v-if=\"showDot\""), "{out}");
    assert!(!out.contains("showDot &&"), "{out}");
}

#[test]
fn variants_switch_on_data_attributes() {
    let mut s = Session::new();
    let mut variants = Vec::new();
    for (k, (state, color)) in [("Default", "#ff0000"), ("Hover", "#0000ff")].iter().enumerate() {
        let f =
            idv(&ex(&mut s, "frame.create", json!({"x": k as f64 * 150.0, "y": 0, "width": 100, "height": 40, "name": format!("State={state}")})));
        ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 40, "parent": f, "props": {"name": "Bg", "fill": color}}));
        variants.push(idv(&ex(&mut s, "component.create", json!({"ids": [f]}))));
    }
    let set = idv(&ex(&mut s, "component.combineVariants", json!({"ids": variants})));
    let out = vue(&mut s, set);
    // A variant property is a string union, bound to the root's data attribute.
    assert!(out.contains("state?: \"Default\" | \"Hover\";"), "{out}");
    assert!(out.contains(":data-state=\"state\""), "{out}");
    // What the other variant changes is a rule on that attribute, as on the web.
    assert!(out.contains("[data-state=\"Hover\"]"), "{out}");
}

#[test]
fn instances_are_calls_passing_the_values_they_set() {
    let mut s = Session::new();
    let (c, _, _) = card(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 400, "y": 0})));
    // Values left at the definition's default aren't passed.
    assert_eq!(component_code(&mut s, i, "vue")["usage"], "<Card />\n");
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Show dot": false, "Title": "Buy"}}));
    // A plain string is an attribute value; anything else is a `:name` binding.
    assert_eq!(component_code(&mut s, i, "vue")["usage"], "<Card :showDot=\"false\" title=\"Buy\" />\n");
    let out = code(&mut s, i, "vue");
    assert!(out.starts_with("<Card :showDot=\"false\" title=\"Buy\" />\n"), "{out}");
    assert!(out.contains("defineProps<{"), "{out}");
}

#[test]
fn a_layer_tree_with_an_instance_calls_it() {
    let mut s = Session::new();
    let (c, _, _) = card(&mut s);
    let parent = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 500, "height": 300, "name": "Page"})));
    ex(&mut s, "instance.create", json!({"component": c, "x": 10, "y": 10, "parent": parent}));
    let out = vue(&mut s, parent);
    assert!(out.contains("<Card "), "{out}");
}

#[test]
fn slots_become_slot_elements() {
    let mut s = Session::new();
    let frame = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Shell"})));
    let body = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "parent": frame, "name": "Body"})));
    let c = idv(&ex(&mut s, "component.create", json!({"ids": [frame]})));
    ex(&mut s, "component.makeSlot", json!({"id": body, "name": "Content"}));
    let out = vue(&mut s, c);
    // An empty slot is Vue's own element, and a slot takes no prop.
    assert!(out.contains("<slot name=\"content\" />"), "{out}");
    assert!(!out.contains("content?: "), "{out}");
    assert!(!out.contains("props"), "{out}");

    // Content in the slot is its fallback, shown until an instance fills it.
    let mut s = Session::new();
    let frame = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Shell2"})));
    let body = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "parent": frame, "name": "Body2"})));
    ex(&mut s, "text.create", json!({"text": "Empty", "x": 0, "y": 0, "parent": body, "props": {"name": "Hint"}}));
    let c = idv(&ex(&mut s, "component.create", json!({"ids": [frame]})));
    ex(&mut s, "component.makeSlot", json!({"id": body, "name": "Content"}));
    let out = vue(&mut s, c);
    assert!(out.contains("<slot name=\"content\">"), "{out}");
    assert!(out.contains("</slot>"), "{out}");
    assert!(out.contains("Empty"), "{out}");
}

#[test]
fn identifiers_are_escaped_and_the_language_round_trips() {
    let mut s = Session::new();
    let (c, _, _) = card(&mut s);
    // "Show dot" is not an identifier: it becomes camelCase in the props and the template alike.
    let out = vue(&mut s, c);
    assert!(out.contains("showDot"), "{out}");
    assert!(!out.contains("Show dot"), "{out}");
    // The id is discoverable from what the Inspect panel offers.
    assert_eq!(CodeLang::from_id("vue"), Some(CodeLang::Vue));
    assert_eq!(CodeLang::from_id("sfc"), Some(CodeLang::Vue));
    assert!(CodeLang::Vue.is_web());
    assert_eq!(CodeLang::Vue.id(), "vue");
    assert!(CodeLang::ALL.contains(&CodeLang::Vue));
}

#[test]
fn rem_converts_lengths_in_the_vue_template_and_its_style() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 320, "height": 64, "name": "Bar"})));
    let out = ex(&mut s, "inspect.code", json!({"id": f, "language": "vue", "unit": "rem"}))["code"].as_str().unwrap_or_default().to_string();
    assert!(out.contains("20rem"), "{out}");
}

#[test]
fn a_keyword_property_name_stays_usable() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Row"})));
    let c = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    // "default" is a JavaScript keyword: the prop is renamed, and the template follows.
    ex(&mut s, "component.addProperty", json!({"id": c, "name": "default", "type": "text", "default": "x"}));
    let out = vue(&mut s, c);
    assert!(out.contains("defaultValue"), "{out}");
}
