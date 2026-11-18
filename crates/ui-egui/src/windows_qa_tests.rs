//! Everyday-workflow bugs found in a Windows QA pass, driven through egui headless: a scrubbed
//! field keeps its value when the pointer lets go, typing on a field Tab reached edits it, and
//! Escape closes a dialog without deselecting.

use egui::{Event, Key, Modifiers, Pos2, Rect, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::a11y::focused_name;
use crate::layers_drag_tests::Ui;

fn press(u: &mut Ui, key: Key, modifiers: Modifiers) {
    u.frame(vec![
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
        Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
    ]);
}

/// A 40×30 rectangle, selected.
fn setup() -> (Ui, NodeId) {
    let mut s = Session::new();
    let a = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let mut u = Ui::new(s);
    u.frame(vec![]);
    (u, a)
}

fn width(u: &Ui, a: NodeId) -> (f64, f64) {
    let n = u.app.session.doc().unwrap().node(a).unwrap();
    (n.width, n.height)
}

/// Give the Design panel the keyboard (F6) and Tab to the field named `name`; its area on screen.
fn tab_to(u: &mut Ui, name: &str) -> Rect {
    press(u, Key::F6, Modifiers::NONE);
    for _ in 0..200 {
        if focused_name(&u.ctx) == name {
            let id = u.ctx.memory(|m| m.focused()).unwrap();
            return u.ctx.viewport(|v| v.prev_pass.widgets.get(id).map(|w| w.rect)).unwrap();
        }
        press(u, Key::Tab, Modifiers::NONE);
    }
    panic!("{name} not reached by Tab");
}

#[test]
fn typing_on_a_field_tab_reached_edits_it() {
    let (mut u, a) = setup();
    tab_to(&mut u, "Width");
    // No Enter first: the typed text replaces the value.
    u.frame(vec![Event::Text("64".into())]);
    press(&mut u, Key::Tab, Modifiers::NONE);
    u.frame(vec![]);
    assert_eq!(focused_name(&u.ctx), "Height");
    u.frame(vec![Event::Text("50".into())]);
    press(&mut u, Key::Enter, Modifiers::NONE);
    u.frame(vec![]);
    assert_eq!(width(&u, a), (64.0, 50.0), "W 64, Tab, H 50, Enter");
}

#[test]
fn a_scrubbed_field_keeps_its_value_when_the_pointer_lets_go() {
    let (mut u, a) = setup();
    let field = tab_to(&mut u, "Width");
    // The label left of the value scrubs.
    let from = Pos2::new(field.min.x - 8.0, field.center().y);
    u.button(from, true);
    for k in 1..=10 {
        u.frame(vec![Event::PointerMoved(from + vec2(4.0 * k as f32, 0.0))]);
    }
    assert_eq!(width(&u, a).0, 60.0, "40 points to the right while dragging: half a step a point");
    u.button(from + vec2(40.0, 0.0), false);
    u.frame(vec![]);
    assert_eq!(width(&u, a).0, 60.0, "kept after the release");
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().height, 30.0);
}

#[test]
fn escape_closes_preferences_without_deselecting() {
    let (mut u, a) = setup();
    // A panel field kept the keyboard when the dialog opened (from the menu or a shortcut).
    tab_to(&mut u, "Width");
    u.app.ui.preferences = true;
    u.frame(vec![]);
    press(&mut u, Key::Escape, Modifiers::NONE);
    assert!(!u.app.ui.preferences, "Escape closes the dialog");
    assert_eq!(u.app.session.selection(), vec![a], "and leaves the selection alone");
    press(&mut u, Key::Escape, Modifiers::NONE);
    assert!(u.app.session.selection().is_empty(), "the next Escape reaches the canvas");
}
