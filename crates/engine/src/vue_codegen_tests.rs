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
    assert!(out.contains("withDefaults(\n  defineProps<{\n"), "{out}");
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

#[test]
fn a_property_named_props_is_not_shadowed() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Card"})));
    let text = idv(&ex(&mut s, "text.create", json!({"text": "Hello", "x": 0, "y": 0, "parent": f, "props": {"name": "Label"}})));
    let c = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    ex(&mut s, "component.addProperty", json!({"id": c, "name": "Props", "type": "text", "layer": text}));
    let out = vue(&mut s, c);
    // No top-level `const props` that the template would resolve to instead of the property.
    assert!(!out.contains("const props"), "{out}");
    assert!(out.contains("props?: string;"), "{out}");
    assert!(out.contains("{{ props }}"), "{out}");
}

#[test]
fn a_nested_instance_keeps_its_v_if_guard() {
    let mut s = Session::new();
    let (card, _, _) = card(&mut s);
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 200, "name": "Toolbar"})));
    ex(&mut s, "layout.add", json!({"ids": [f], "direction": "vertical"}));
    let nested = idv(&ex(&mut s, "instance.create", json!({"component": card, "x": 10, "y": 10, "parent": f})));
    let c = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    ex(&mut s, "component.addProperty", json!({"id": c, "name": "Show card", "type": "boolean", "default": true, "layer": nested}));
    ex(&mut s, "instance.setProperties", json!({"ids": [nested], "properties": {"Title": "a > b"}}));
    let out = vue(&mut s, c);
    // The call's early return must not swallow the guard `node` put on it, and the `>` inside a
    // quoted value must not be mistaken for the tag's end.
    assert!(out.contains("v-if=\"showCard\""), "{out}");
    assert_eq!(out.matches("v-if=\"showCard\"").count(), 1, "{out}");
    assert!(!out.contains("/ v-if"), "the guard must come before a self-closing slash: {out}");
    assert!(out.contains("title=\"a &gt; b\"") || out.contains("title=\"a > b\""), "{out}");
}

#[test]
fn a_text_default_cannot_close_the_script_block() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Card"})));
    let text = idv(&ex(&mut s, "text.create", json!({"text": "Hello", "x": 0, "y": 0, "parent": f, "props": {"name": "Title"}})));
    let c = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    ex(
        &mut s,
        "component.addProperty",
        json!({"id": c, "name": "Title", "type": "text", "layer": text, "default": "</script><script>alert(1)</script>"}),
    );
    let out = vue(&mut s, c);
    assert!(out.contains("\\u003C/script>"), "{out}");
    assert_eq!(out.matches("</script>").count(), 1, "only the SFC's own closing tag: {out}");
}

#[test]
fn slot_names_stay_distinct_when_idents_lowercase_alike() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 200, "name": "Shell"})));
    let a = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100, "parent": f, "name": "A"})));
    let b = idv(&ex(&mut s, "frame.create", json!({"x": 100, "y": 0, "width": 100, "height": 100, "parent": f, "name": "B"})));
    let c = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    ex(&mut s, "component.makeSlot", json!({"id": a, "name": "Foo Bar"}));
    ex(&mut s, "component.makeSlot", json!({"id": b, "name": "Foobar"}));
    let out = vue(&mut s, c);
    assert!(out.contains("<slot name=\"foobar\""), "{out}");
    assert!(out.contains("<slot name=\"foobar-2\""), "{out}");
    // The instance's templates use the same names.
    let g = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 300, "width": 20, "height": 20, "name": "Badge"})));
    let badge = idv(&ex(&mut s, "component.create", json!({"ids": [g]})));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 400, "y": 0})));
    ex(&mut s, "instance.slotInsert", json!({"id": i, "slot": "Foo Bar", "component": badge, "replace": true}));
    ex(&mut s, "instance.slotInsert", json!({"id": i, "slot": "Foobar", "component": badge, "replace": true}));
    let usage = component_code(&mut s, i, "vue")["usage"].as_str().unwrap_or_default().to_string();
    assert!(usage.contains("<template #foobar>"), "{usage}");
    assert!(usage.contains("<template #foobar-2>"), "{usage}");
}

#[test]
fn slot_content_css_reaches_the_parent_stylesheet() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 200, "name": "Panel"})));
    let content = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "parent": f, "name": "Content"})));
    let panel = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    ex(&mut s, "component.makeSlot", json!({"id": content, "name": "Content"}));
    let screen = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 400, "width": 400, "height": 300, "name": "Screen"})));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": panel, "x": 10, "y": 10, "parent": screen})));
    let slot = {
        let doc = s.doc().unwrap();
        crate::components::slots_of(doc, i).first().map(|(l, _)| *l).unwrap()
    };
    ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10, "parent": slot, "props": {"name": "Swatch", "fill": "#FF0000"}}));
    // The layer's class rules belong to the tree that holds the instance, not to a discarded
    // snippet: both web targets keep them in their stylesheet.
    let out = vue(&mut s, screen);
    assert!(out.contains(".slot-content-swatch {"), "{out}");
    assert!(out.contains("#FF0000"), "{out}");
    assert!(code(&mut s, screen, "react").contains(".slot-content-swatch {"));
}

#[test]
fn slot_content_classes_are_unique_across_slots() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 200, "name": "Panel"})));
    let one = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100, "parent": f, "name": "One"})));
    let two = idv(&ex(&mut s, "frame.create", json!({"x": 100, "y": 0, "width": 100, "height": 100, "parent": f, "name": "Two"})));
    let panel = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    ex(&mut s, "component.makeSlot", json!({"id": one, "name": "One"}));
    ex(&mut s, "component.makeSlot", json!({"id": two, "name": "Two"}));
    let screen = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 400, "width": 400, "height": 300, "name": "Screen"})));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": panel, "x": 10, "y": 10, "parent": screen})));
    let slots = {
        let doc = s.doc().unwrap();
        crate::components::slots_of(doc, i)
    };
    for (name, fill) in [("One", "#FF0000"), ("Two", "#0000FF")] {
        let slot = slots.iter().find(|(_, n)| n.as_str() == name).map(|(l, _)| *l).unwrap();
        ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10, "parent": slot, "props": {"name": "Swatch", "fill": fill}}));
    }
    let out = vue(&mut s, screen);
    // Same layer name in two slots: each slot's rules keep their own class.
    assert!(out.contains(".slot-one-swatch {"), "{out}");
    assert!(out.contains(".slot-two-swatch {"), "{out}");
    assert!(out.contains("#FF0000"), "{out}");
    assert!(out.contains("#0000FF"), "{out}");
}
