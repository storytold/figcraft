//! A double-click on the empty canvas, driven through egui headless: the gesture its second press
//! began is dropped, so the tool is idle once the button is up.

use egui::{Event, Modifiers, PointerButton, Pos2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

/// Two presses and releases at the bottom left of the canvas, with `modifiers` held.
fn double_click(u: &mut Ui, modifiers: Modifiers) {
    let pos = Pos2::new(u.app.canvas.rect.left() + 30.0, u.app.canvas.rect.bottom() - 30.0);
    u.frame_with(vec![Event::PointerMoved(pos)], modifiers);
    for _ in 0..2 {
        for pressed in [true, false] {
            u.frame_with(vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }], modifiers);
        }
    }
    u.frame_with(vec![], modifiers);
}

#[test]
fn a_double_click_leaves_the_shape_tool_idle() {
    let mut s = Session::new();
    s.execute("pref.keepTool", &json!({"on": true})).unwrap();
    s.execute("tool.set", &json!({"tool": "rectangle"})).unwrap();
    let mut u = Ui::new(s);
    assert_eq!(u.app.inspect()["tool"]["busy"], false);
    double_click(&mut u, Modifiers::default());
    assert_eq!(u.app.inspect()["tool"]["busy"], false, "a finished double-click leaves the tool idle");
    assert!(u.app.session.execute("tool.place", &json!({})).is_ok(), "Enter places a shape");
}

#[test]
fn a_shift_double_click_keeps_the_selection_handles() {
    let mut s = Session::new();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 300, "height": 200})).unwrap();
    let r = figcraft_engine::params::id_of(&r["id"]).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    let handles = |u: &Ui| u.app.inspect()["tool"]["shapeHandles"].as_array().map_or(0, Vec::len);
    assert_eq!(handles(&u), 4);
    double_click(&mut u, Modifiers::SHIFT);
    assert_eq!(u.app.session.selection(), vec![r], "Shift keeps the selection");
    assert_eq!(u.app.inspect()["tool"]["busy"], false, "a finished double-click leaves the tool idle");
    assert_eq!(handles(&u), 4, "the corner radius handles show again");
}
