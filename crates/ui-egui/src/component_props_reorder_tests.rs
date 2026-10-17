//! Dragging variant properties in a component set's Properties section, driven through egui
//! headless: a property dragged by its left margin moves in one undo step (its values go with it,
//! instances follow), its fields don't start a drag, and Escape cancels.

use egui::{Event, Pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::list_drag::grip_id;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A selected set whose variants list Size then State (two values each), a shared boolean "Icon",
/// and an instance of the set. Returns (ui, set, instance).
fn setup() -> (Ui, NodeId, NodeId) {
    let mut s = Session::new();
    let mut variants = Vec::new();
    for (k, name) in ["Size=Small, State=Default", "Size=Large, State=Default", "Size=Small, State=Hover"].iter().enumerate() {
        let f = id(s.execute("frame.create", &json!({"x": 0, "y": k * 60, "width": 80, "height": 40, "name": name})).unwrap());
        variants.push(id(s.execute("component.create", &json!({"ids": [f]})).unwrap()));
    }
    let set = id(s.execute("component.combineVariants", &json!({"ids": variants})).unwrap());
    s.execute("component.addProperty", &json!({"id": set, "name": "Icon", "type": "boolean", "default": true})).unwrap();
    let inst = id(s.execute("instance.create", &json!({"component": set, "x": 400, "y": 0})).unwrap());
    s.execute("select.set", &json!({"ids": [set]})).unwrap();
    (Ui::new(s), set, inst)
}

/// Property names of `id` (a set or an instance), in the order they're listed.
fn names(u: &mut Ui, id: NodeId) -> Vec<String> {
    let props = u.app.session.execute("component.properties", &json!({"id": id})).unwrap();
    props["properties"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect()
}

fn undo_len(u: &Ui) -> usize {
    u.app.session.ds().unwrap().history.undo.len()
}

/// The middle of the grip of variant property `index` (as laid out last frame).
fn grip(u: &Ui, set: NodeId, index: usize) -> Option<Pos2> {
    u.ctx.read_response(grip_id(("variant_props", set), index)).map(|r| r.rect.center())
}

/// Press at `from`, move to `to` in small steps, and leave the button down.
fn drag(u: &mut Ui, from: Pos2, to: Pos2) {
    u.button(from, true);
    for k in 1..=10 {
        u.frame(vec![Event::PointerMoved(from + (to - from) * (k as f32 / 10.0))]);
    }
}

#[test]
fn dragging_a_variant_property_reorders_it_in_one_step() {
    let (mut u, set, inst) = setup();
    assert_eq!(names(&mut u, set), ["Size", "State", "Icon"]);
    let (size, state) = (grip(&u, set, 0).expect("variant properties have a grip"), grip(&u, set, 1).unwrap());
    assert!(size.y < state.y, "listed in order");
    let undo = undo_len(&u);
    // State above Size.
    let to = size - vec2(0.0, 12.0);
    drag(&mut u, state, to);
    assert_eq!(names(&mut u, set), ["Size", "State", "Icon"], "nothing moves before the drop");
    u.button(to, false);
    assert_eq!(names(&mut u, set), ["State", "Size", "Icon"]);
    assert_eq!(names(&mut u, inst), ["State", "Size", "Icon"], "the instance lists the new order");
    let doc = u.app.session.doc().unwrap();
    let first = doc.node(set).unwrap().children[0];
    assert!(doc.node(first).unwrap().name.starts_with("State="), "{}", doc.node(first).unwrap().name);
    assert_eq!(undo_len(&u), undo + 1);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    // Dropped where it is: nothing changes.
    u.frame(vec![]);
    u.frame(vec![]);
    let g = grip(&u, set, 0).unwrap();
    drag(&mut u, g, g + vec2(0.0, 6.0));
    u.button(g + vec2(0.0, 6.0), false);
    assert_eq!(undo_len(&u), undo + 1);
    assert!(u.app.session.undo().unwrap());
    assert_eq!(names(&mut u, set), ["Size", "State", "Icon"]);
}

#[test]
fn a_property_dropped_under_the_last_ones_values_lands_last() {
    let (mut u, set, _) = setup();
    let size = grip(&u, set, 0).unwrap();
    // State's header is 32 high and its value row 30 under it: 58 down is on the value row, below
    // the header's half-row slack, so only an area that takes in the values counts it.
    let state = u.ctx.read_response(grip_id(("variant_props", set), 1)).unwrap().rect;
    let to = Pos2::new(size.x, state.min.y + 58.0);
    drag(&mut u, size, to);
    u.button(to, false);
    assert_eq!(names(&mut u, set), ["State", "Size", "Icon"]);
}

#[test]
fn pressing_the_name_field_does_not_start_a_drag() {
    // A guard: on main there's no grip at all; it fails if the whole row becomes a drag handle.
    let (mut u, set, _) = setup();
    let (size, state) = (grip(&u, set, 0).unwrap(), grip(&u, set, 1).unwrap());
    let undo = undo_len(&u);
    let field = size + vec2(60.0, 0.0);
    drag(&mut u, field, Pos2::new(field.x, state.y + 40.0));
    u.button(Pos2::new(field.x, state.y + 40.0), false);
    assert_eq!(names(&mut u, set), ["Size", "State", "Icon"]);
    assert_eq!(undo_len(&u), undo);
}

#[test]
fn escape_cancels_a_property_drag_and_keeps_the_selection() {
    // A guard: the cancel comes from the shared list drag.
    let (mut u, set, _) = setup();
    let (size, state) = (grip(&u, set, 0).unwrap(), grip(&u, set, 1).unwrap());
    let undo = undo_len(&u);
    let to = size - vec2(0.0, 12.0);
    drag(&mut u, state, to);
    u.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    u.button(to, false);
    assert_eq!(names(&mut u, set), ["Size", "State", "Icon"]);
    assert_eq!(undo_len(&u), undo);
    assert_eq!(u.app.session.ds().unwrap().selection, [set], "the Escape only cancels the drag");
}
