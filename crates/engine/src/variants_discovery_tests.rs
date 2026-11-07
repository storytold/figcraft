//! Combine as variants and Add variant where people look for them (#903): the predicates the
//! context menu and the Design panel offer them by, combining refusing components that are
//! variants already (it used to put a set inside a set), and Add variant on a main component that
//! isn't in a set yet making it one.

use serde_json::json;

use crate::Session;
use crate::cmd::component::{can_add_variant, can_combine_variants};
use crate::components::tests::{ex, healthy, idv, kids, node};
use figcraft_doc::*;

/// A main component named `name` at `x`, made from a 120×40 frame with a "Bg" rectangle.
fn component(s: &mut Session, name: &str, x: f64) -> NodeId {
    let f = idv(&ex(s, "frame.create", json!({"x": x, "y": 0, "width": 120, "height": 40, "name": name})));
    ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 120, "height": 40, "parent": f, "props": {"name": "Bg"}}));
    idv(&ex(s, "component.create", json!({"ids": [f]})))
}

fn undo_len(s: &Session) -> usize {
    s.ds().unwrap().history.undo.len()
}

fn sets(s: &Session) -> usize {
    s.doc().unwrap().nodes().filter(|n| matches!(n.kind, NodeKind::ComponentSet { .. })).count()
}

#[test]
fn combine_is_offered_for_two_or_more_main_components_only() {
    let mut s = Session::new();
    let a = component(&mut s, "A", 0.0);
    let b = component(&mut s, "B", 200.0);
    let frame = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 300, "width": 50, "height": 50})));
    let inst = idv(&ex(&mut s, "instance.create", json!({"component": a, "x": 0, "y": 400})));
    let doc = s.doc().unwrap();
    assert!(can_combine_variants(doc, &[a, b]));
    assert!(!can_combine_variants(doc, &[]), "nothing selected");
    assert!(!can_combine_variants(doc, &[a]), "one component (the command still takes it)");
    assert!(!can_combine_variants(doc, &[a, a]), "the same component twice");
    assert!(!can_combine_variants(doc, &[a, frame]), "a frame isn't a component");
    assert!(!can_combine_variants(doc, &[a, inst]), "an instance isn't a main component");

    let c = component(&mut s, "C", 400.0);
    let d = component(&mut s, "D", 600.0);
    let set = idv(&ex(&mut s, "component.combineVariants", json!({"ids": [c, d]})));
    let doc = s.doc().unwrap();
    assert!(!can_combine_variants(doc, &[c, d]), "variants of one set");
    assert!(!can_combine_variants(doc, &[c, a]), "a variant and a lone component");
    assert!(!can_combine_variants(doc, &[set, a]), "a set isn't a component");

    // Components in different frames still combine.
    let holder = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 600, "width": 300, "height": 100})));
    let e = component(&mut s, "E", 0.0);
    ex(&mut s, "node.reparent", json!({"ids": [e], "parent": holder}));
    assert!(can_combine_variants(s.doc().unwrap(), &[a, e]));
}

#[test]
fn add_variant_is_offered_for_sets_variants_and_lone_components() {
    let mut s = Session::new();
    let a = component(&mut s, "A", 0.0);
    let b = component(&mut s, "B", 200.0);
    let lone = component(&mut s, "Lone", 400.0);
    let set = idv(&ex(&mut s, "component.combineVariants", json!({"ids": [a, b]})));
    let frame = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 300, "width": 50, "height": 50})));
    let inst = idv(&ex(&mut s, "instance.create", json!({"component": lone, "x": 0, "y": 400})));
    let doc = s.doc().unwrap();
    assert!(can_add_variant(doc, set));
    assert!(can_add_variant(doc, a));
    assert!(can_add_variant(doc, lone));
    assert!(!can_add_variant(doc, frame));
    assert!(!can_add_variant(doc, inst));
    assert!(!can_add_variant(doc, kids(&s, inst)[0]), "a layer inside an instance");
    assert!(!can_add_variant(doc, NodeId(999_999)));
}

#[test]
fn combining_variants_again_is_refused_instead_of_nesting_sets() {
    let mut s = Session::new();
    let a = component(&mut s, "State=Default", 0.0);
    let b = component(&mut s, "State=Hover", 200.0);
    let lone = component(&mut s, "State=Pressed", 400.0);
    let set = idv(&ex(&mut s, "component.combineVariants", json!({"ids": [a, b]})));
    let before = s.doc().unwrap().clone();
    let steps = undo_len(&s);
    for ids in [vec![a, b], vec![a, lone]] {
        let err = s.execute("component.combineVariants", &json!({"ids": ids})).unwrap_err();
        assert!(err.to_string().contains("variants already"), "{err}");
    }
    assert_eq!(*s.doc().unwrap(), before, "a refused combine changes nothing");
    assert_eq!(undo_len(&s), steps);
    assert_eq!(sets(&s), 1);
    assert_eq!(kids(&s, set), vec![a, b]);
    // One lone component still makes a set of one (other commands rely on it).
    ex(&mut s, "component.combineVariants", json!({"ids": [lone]}));
    assert_eq!(sets(&s), 2);
    healthy(&s);
}

#[test]
fn combining_refuses_nested_components_and_components_inside_instances() {
    let mut s = Session::new();
    // A "Card" component holding an "Icon" component.
    let card = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Card"})));
    let icon = idv(&ex(&mut s, "frame.create", json!({"x": 10, "y": 10, "width": 20, "height": 20, "name": "Icon", "parent": card})));
    let icon = idv(&ex(&mut s, "component.create", json!({"ids": [icon]})));
    let card = idv(&ex(&mut s, "component.create", json!({"ids": [card]})));
    let other = component(&mut s, "Other", 300.0);
    let inst = idv(&ex(&mut s, "instance.create", json!({"component": card, "x": 0, "y": 300})));
    let inner = kids(&s, inst).into_iter().find(|c| matches!(node(&s, *c).kind, NodeKind::Component { .. })).expect("the instance's copy of Icon");

    let doc = s.doc().unwrap();
    assert!(!can_combine_variants(doc, &[card, icon]), "a component and one inside it");
    assert!(can_combine_variants(doc, &[icon, other]), "a nested component with another one is fine");
    assert!(!can_combine_variants(doc, &[inner, other]), "a component inside an instance");
    let before = s.doc().unwrap().clone();
    let err = s.execute("component.combineVariants", &json!({"ids": [card, icon]})).unwrap_err();
    assert!(err.to_string().contains("inside it"), "{err}");
    assert_eq!(*s.doc().unwrap(), before);
}

#[test]
fn add_variant_on_a_lone_component_makes_it_a_set_in_one_step() {
    let mut s = Session::new();
    let c = component(&mut s, "Button", 0.0);
    let inst = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 0, "y": 300})));
    let before = s.doc().unwrap().clone();
    let steps = undo_len(&s);

    let v = idv(&ex(&mut s, "component.addVariant", json!({"id": c})));
    let set = node(&s, c).parent.unwrap();
    let set_node = node(&s, set);
    assert!(matches!(set_node.kind, NodeKind::ComponentSet { .. }));
    assert_eq!(set_node.name, "Button", "the set takes the component's name");
    assert_eq!(kids(&s, set), vec![c, v]);
    assert_eq!(node(&s, c).name, "Property 1=Default");
    assert_eq!(node(&s, v).name, "Property 1=Variant2");
    assert_eq!(s.selection(), vec![v]);
    assert!(set_node.height >= 40.0 * 2.0 + 20.0, "the set wraps both variants: {}", set_node.height);
    assert!(matches!(node(&s, inst).kind, NodeKind::Instance { component, .. } if component == c), "instances keep their main");
    assert_eq!(undo_len(&s), steps + 1, "wrapping and adding are one undo step");
    healthy(&s);

    s.undo().unwrap();
    assert_eq!(*s.doc().unwrap(), before, "undo puts the lone component back");
}

#[test]
fn add_variant_on_a_lone_component_keeps_prop_value_names_and_rolls_back_on_errors() {
    let mut s = Session::new();
    let c = component(&mut s, "Size=Small", 0.0);
    let before = s.doc().unwrap().clone();
    let steps = undo_len(&s);
    assert!(s.execute("component.addVariant", &json!({"id": c, "property": "Nope", "value": "x"})).is_err());
    assert_eq!(*s.doc().unwrap(), before, "a refused Add variant leaves no set behind");
    assert_eq!(undo_len(&s), steps);

    let v = idv(&ex(&mut s, "component.addVariant", json!({"id": c, "property": "Size", "value": "Large"})));
    assert_eq!(node(&s, c).name, "Size=Small");
    assert_eq!(node(&s, v).name, "Size=Large");
    healthy(&s);
}

#[test]
fn add_variant_on_a_lone_component_in_auto_layout_keeps_its_place() {
    let mut s = Session::new();
    let row = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 100, "name": "Row"})));
    ex(&mut s, "layout.add", json!({"ids": [row]}));
    let first = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 40, "parent": row})));
    let c = component(&mut s, "Chip", 0.0);
    ex(&mut s, "node.reparent", json!({"ids": [c], "parent": row}));
    let last = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 40, "parent": row})));

    ex(&mut s, "component.addVariant", json!({"id": c}));
    let set = node(&s, c).parent.unwrap();
    assert_eq!(kids(&s, row), vec![first, set, last], "the set takes the component's place in the row");
    assert_eq!(kids(&s, set).len(), 2);
    healthy(&s);
}
