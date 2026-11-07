//! Make slot asks for the slot's name first, driven through egui headless (#1537): picking it in
//! the Slot menu of a frame inside a component shows a name field with the frame's name selected;
//! what is typed names the slot on Enter, and Escape makes nothing.

use egui::Event;
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use figcraft_engine::components::slot_name;
use serde_json::json;

use crate::component_swap_default_tests::{centre, click, tree};
use crate::layers_drag_tests::Ui;

/// A "Card" component holding a selected frame "Frame 12". Returns (ui, frame).
fn setup() -> (Ui, NodeId) {
    let mut s = Session::new();
    let id = |v: serde_json::Value| NodeId(v["id"].as_u64().unwrap());
    let card = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 200, "name": "Card"})).unwrap());
    let frame = id(s.execute("frame.create", &json!({"x": 10, "y": 50, "width": 200, "height": 100, "parent": card, "name": "Frame 12"})).unwrap());
    s.execute("component.create", &json!({"ids": [card]})).unwrap();
    s.execute("select.set", &json!({"ids": [frame]})).unwrap();
    let u = Ui::new(s);
    u.ctx.enable_accesskit();
    (u, frame)
}

fn key(key: egui::Key) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
}

/// Open the frame's Slot menu and pick Make slot; returns the name field's value then.
fn pick_make_slot(u: &mut Ui) -> Option<String> {
    let combo = tree(u).into_iter().find(|n| n.label() == Some("Slot") && n.value() == Some("Not bound")).expect("the frame has a Slot row");
    click(u, centre(&combo));
    let item = tree(u).into_iter().find(|n| n.label() == Some("Make slot")).expect("the Slot menu offers Make slot");
    click(u, centre(&item));
    u.frame(vec![]);
    u.frame(vec![]);
    tree(u).into_iter().find(|n| n.label() == Some("Slot name")).and_then(|n| n.value().map(str::to_string))
}

fn slot_of(u: &Ui, frame: NodeId) -> Option<String> {
    slot_name(u.app.session.doc().unwrap(), frame)
}

#[test]
fn make_slot_names_the_slot_with_what_is_typed() {
    let (mut u, frame) = setup();
    assert_eq!(pick_make_slot(&mut u).as_deref(), Some("Frame 12"), "the field suggests the frame's name");
    assert_eq!(slot_of(&u, frame), None, "nothing is made before the name is confirmed");
    // The suggestion is selected: typing replaces it.
    u.frame(vec![Event::Text("Avatar".into())]);
    u.frame(vec![key(egui::Key::Enter)]);
    u.frame(vec![]);
    assert_eq!(slot_of(&u, frame).as_deref(), Some("Avatar"));
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    assert!(tree(&mut u).iter().all(|n| n.label() != Some("Slot name")), "the naming row closes");
}

#[test]
fn escape_cancels_make_slot() {
    let (mut u, frame) = setup();
    assert!(pick_make_slot(&mut u).is_some());
    u.frame(vec![key(egui::Key::Escape)]);
    u.frame(vec![]);
    assert_eq!(slot_of(&u, frame), None);
    assert!(tree(&mut u).iter().all(|n| n.label() != Some("Slot name")));
    assert_eq!(u.app.session.ds().unwrap().selection, [frame], "the Escape only cancels the naming");
}

#[test]
fn a_name_taken_keeps_the_field_open() {
    let (mut u, frame) = setup();
    let card = u.app.session.doc().unwrap().node(frame).and_then(|n| n.parent).unwrap();
    u.app.session.execute("component.addProperty", &json!({"id": card, "name": "Taken", "type": "boolean"})).unwrap();
    assert!(pick_make_slot(&mut u).is_some());
    u.frame(vec![Event::Text("Taken".into())]);
    u.frame(vec![key(egui::Key::Enter)]);
    u.frame(vec![]);
    assert_eq!(slot_of(&u, frame), None);
    let field = tree(&mut u).into_iter().find(|n| n.label() == Some("Slot name")).expect("the field stays to fix the name");
    assert_eq!(field.value(), Some("Taken"));
}
