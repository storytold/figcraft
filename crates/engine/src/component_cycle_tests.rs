//! Components can't contain their own instances, directly, through other components or through
//! the variants of a set (#1084); files holding such a loop open with it broken.

use std::sync::Arc;

use base64::Engine;
use figcraft_doc::{Document, NodeId, NodeKind};
use serde_json::{Value, json};

use crate::Session;
use crate::components::component_cycle;
use crate::components::tests::{copy_of, ex, idv, kids, node};

/// A 100×50 component named `name` at (x, 0), holding a rectangle; returns its id.
fn component(s: &mut Session, name: &str, x: f64) -> NodeId {
    let f = idv(&ex(s, "frame.create", json!({"x": x, "y": 0, "width": 100, "height": 50, "name": name})));
    ex(s, "shape.rectangle", json!({"x": 5, "y": 5, "width": 10, "height": 10, "parent": f}));
    idv(&ex(s, "component.create", json!({"ids": [f]})))
}

fn instance(s: &mut Session, component: NodeId, parent: NodeId) -> NodeId {
    idv(&ex(s, "instance.create", json!({"component": component, "x": 20, "y": 20, "parent": parent})))
}

/// `cmd` is refused with the loop explained, and the document is left as it was.
fn refused(s: &mut Session, cmd: &str, p: Value) {
    let before = s.doc().unwrap().clone();
    let undo = s.ds().unwrap().history.undo.len();
    let err = s.execute(cmd, &p).expect_err(&format!("{cmd} {p} should be refused")).to_string();
    assert!(err.contains("can't contain"), "{cmd}: {err}");
    assert_eq!(*s.doc().unwrap(), before, "{cmd} rolled back");
    assert_eq!(s.ds().unwrap().history.undo.len(), undo, "{cmd} left no undo step");
}

/// Card holds a Badge instance; Badge holds an Icon instance. Returns (card, badge, icon, the
/// Icon instance in Badge).
fn chain(s: &mut Session) -> (NodeId, NodeId, NodeId, NodeId) {
    let icon = component(s, "Icon", 0.0);
    let badge = component(s, "Badge", 200.0);
    let card = component(s, "Card", 400.0);
    let in_badge = instance(s, icon, badge);
    instance(s, badge, card);
    (card, badge, icon, in_badge)
}

#[test]
fn direct_and_indirect_loops_are_refused() {
    let mut s = Session::new();
    let (card, badge, _, in_badge) = chain(&mut s);
    // Directly, and through Badge: Card already shows Badge.
    refused(&mut s, "instance.create", json!({"component": card, "parent": card}));
    refused(&mut s, "instance.create", json!({"component": card, "parent": badge}));
    refused(&mut s, "instance.swap", json!({"ids": [in_badge], "component": card}));
    // A Card instance moved or pasted into Badge.
    let page = s.page().unwrap();
    let loose = instance(&mut s, card, page);
    refused(&mut s, "node.reparent", json!({"ids": [loose], "parent": badge}));
    ex(&mut s, "edit.copy", json!({"ids": [loose]}));
    refused(&mut s, "edit.paste", json!({"parent": badge}));
    // A copied main pastes as an instance of it.
    ex(&mut s, "edit.copy", json!({"ids": [card]}));
    refused(&mut s, "edit.paste", json!({"parent": badge}));
    assert!(component_cycle(s.doc().unwrap()).is_none());
}

#[test]
fn dropping_an_instance_where_it_would_contain_itself_places_it_on_the_page() {
    let mut s = Session::new();
    let (card, badge, _, _) = chain(&mut s);
    let r = ex(&mut s, "instance.place", json!({"component": card, "x": 250, "y": 25}));
    assert_eq!(r["parent"], json!(s.page().unwrap()), "not into Badge: {r}");
    assert!(!s.doc().unwrap().is_ancestor(badge, idv(&r)));
}

#[test]
fn an_instance_swap_property_can_not_close_a_loop() {
    let mut s = Session::new();
    let (card, badge, icon, in_badge) = chain(&mut s);
    ex(&mut s, "component.addProperty", json!({"id": badge, "name": "Icon", "type": "instanceSwap", "default": icon, "layer": in_badge}));
    let in_card = kids(&s, card).into_iter().find(|k| matches!(node(&s, *k).kind, NodeKind::Instance { .. })).unwrap();
    refused(&mut s, "instance.setProperties", json!({"ids": [in_card], "properties": {"Icon": card}}));
    // Another component is fine.
    let star = component(&mut s, "Star", 600.0);
    ex(&mut s, "instance.setProperties", json!({"ids": [in_card], "properties": {"Icon": star}}));
}

/// A set of two variants, "State=On" holding an instance of "State=Off". Returns (on, off, the Off
/// instance in On).
fn nested_variants(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let on = component(s, "State=On", 0.0);
    let off = component(s, "State=Off", 200.0);
    ex(s, "component.combineVariants", json!({"ids": [on, off]}));
    let held = instance(s, off, on);
    (on, off, held)
}

#[test]
fn a_variant_holds_a_sibling_but_no_loop_back_through_it() {
    let mut s = Session::new();
    let (on, off, held) = nested_variants(&mut s);
    assert!(component_cycle(s.doc().unwrap()).is_none(), "a sibling variant's instance is no loop");
    // Off leading back to On, directly or through another component, is one.
    refused(&mut s, "instance.create", json!({"component": on, "parent": off}));
    let row = component(&mut s, "Row", 600.0);
    instance(&mut s, on, row);
    refused(&mut s, "instance.create", json!({"component": row, "parent": off}));
    // So is switching the held instance to the variant holding it.
    refused(&mut s, "instance.setProperties", json!({"ids": [held], "properties": {"State": "On"}}));
    // Combining a component with one holding its instance is fine too.
    let a = component(&mut s, "A", 0.0);
    let b = component(&mut s, "B", 200.0);
    instance(&mut s, a, b);
    ex(&mut s, "component.combineVariants", json!({"ids": [a, b]}));
    assert!(component_cycle(s.doc().unwrap()).is_none());
}

/// `doc` saved and opened again: the session and what opening it reported.
fn reopened(doc: &Document) -> (Session, Value) {
    let data = base64::engine::general_purpose::STANDARD.encode(figcraft_format::save_bytes(doc, None).unwrap());
    let mut s = Session::new();
    let r = ex(&mut s, "file.open", json!({"data": data, "name": "variants.figcraft"}));
    (s, r)
}

#[test]
fn a_file_with_nested_sibling_variants_opens_unchanged_and_a_loop_through_them_is_repaired() {
    let mut s = Session::new();
    let (on, off, held) = nested_variants(&mut s);
    let doc = s.doc().unwrap().clone();
    let (t, r) = reopened(&doc);
    assert_eq!(r["repairs"], json!([]), "{r}");
    assert_eq!(t.doc().unwrap().nodes().count(), doc.nodes().count());
    assert!(matches!(node(&t, held).kind, NodeKind::Instance { component, .. } if component == off));
    // On → Off → On: an On instance in Off, written directly as an older file could have it.
    let spare = component(&mut s, "Spare", 600.0);
    let back = instance(&mut s, spare, off);
    let mut doc = s.doc().unwrap().clone();
    if let NodeKind::Instance { component, .. } = &mut doc.get_mut(back).unwrap().kind {
        *component = on;
    }
    assert!(component_cycle(&doc).is_some());
    let (t, r) = reopened(&doc);
    assert!(r["repairs"].as_array().is_some_and(|n| n.iter().any(|n| n.as_str().is_some_and(|n| n.contains("was detached")))), "{r}");
    assert!(component_cycle(t.doc().unwrap()).is_none());
}

#[test]
fn components_around_instances_and_instances_in_slots_stay_allowed() {
    let mut s = Session::new();
    let (card, badge, ..) = chain(&mut s);
    // A new component around a Card instance.
    let page = s.page().unwrap();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 300, "width": 200, "height": 100, "parent": page})));
    instance(&mut s, card, f);
    let holder = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    instance(&mut s, badge, holder);
    // An instance in a slot of an instance of its own component: the slot holds that instance's
    // own content, outside its main.
    let slot = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 50, "height": 20, "parent": card, "name": "Slot"})));
    ex(&mut s, "component.makeSlot", json!({"id": slot}));
    let outer = instance(&mut s, card, page);
    let outer_slot = copy_of(&s, outer, slot);
    instance(&mut s, card, outer_slot);
    assert!(component_cycle(s.doc().unwrap()).is_none());
}

/// A document where Card holds a Badge instance and Badge a Card instance, made by editing the
/// layers directly (as an older file or a merge could have it). Returns it with (card, badge).
fn looped() -> (Document, NodeId, NodeId) {
    let mut s = Session::new();
    let (card, badge, _, in_badge) = chain(&mut s);
    let mut doc = s.doc().unwrap().clone();
    if let NodeKind::Instance { component, .. } = &mut doc.get_mut(in_badge).unwrap().kind {
        *component = card;
    }
    crate::components::sync_instance(&mut doc, in_badge);
    assert!(component_cycle(&doc).is_some());
    (doc, card, badge)
}

/// Edit mains `a` and `b` `n` times, every instance rebuilt in full each time; the layer count
/// after each round.
fn edit_mains(s: &mut Session, a: NodeId, b: NodeId, n: usize) -> Vec<usize> {
    crate::components::FULL_REBUILD.with(|f| f.set(true));
    let sizes = (0..n)
        .map(|i| {
            ex(s, "node.set", json!({"id": a, "props": {"fill": format!("#{:02x}0000", i * 20)}}));
            ex(s, "node.set", json!({"id": b, "props": {"fill": format!("#00{:02x}00", i * 20)}}));
            s.doc().unwrap().nodes().count()
        })
        .collect();
    crate::components::FULL_REBUILD.with(|f| f.set(false));
    sizes
}

#[test]
fn a_file_with_a_loop_opens_with_it_broken_and_a_note() {
    let (doc, card, badge) = looped();
    let bytes = figcraft_format::save_bytes(&doc, None).unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let mut s = Session::new();
    let r = ex(&mut s, "file.open", json!({"data": data, "name": "looped.figcraft"}));
    let notes = r["repairs"].as_array().cloned().unwrap_or_default();
    assert!(notes.iter().any(|n| n.as_str().is_some_and(|n| n.contains("was detached"))), "{r}");
    let doc = s.doc().unwrap();
    assert!(component_cycle(doc).is_none());
    // The detached instance kept its layers; the other side is still an instance.
    assert!(doc.contains(card) && doc.contains(badge));
    let count = doc.nodes().count();
    // Editing the mains no longer grows the file.
    assert_eq!(edit_mains(&mut s, card, badge, 10), vec![count; 10]);
    // Importing it into another file breaks it the same way.
    let mut t = Session::new();
    let r = ex(&mut t, "file.import", json!({"data": data, "name": "looped.figcraft"}));
    assert!(r["warnings"].as_array().is_some_and(|w| !w.is_empty()), "{r}");
    assert!(component_cycle(t.doc().unwrap()).is_none());
}

#[test]
fn a_loop_merged_in_stops_expanding_and_the_document_stays_editable() {
    let (doc, card, badge) = looped();
    let mut s = Session::new();
    let uid = s.ds().unwrap().uid;
    s.apply_remote(uid, Arc::new(doc));
    assert!(s.message.as_deref().is_some_and(|m| m.contains("contain each other")), "{:?}", s.message);
    let sizes = edit_mains(&mut s, card, badge, 12);
    // A couple of rebuilds settle the copies; after that nothing grows.
    assert!(sizes.windows(2).skip(2).all(|w| w[0] == w[1]), "{sizes:?}");
    // Editing goes on: layers added to a main reach its instances.
    ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 4, "height": 4, "parent": card}));
    let page = s.page().unwrap();
    let loose = instance(&mut s, badge, page);
    assert!(!kids(&s, loose).is_empty());
}
