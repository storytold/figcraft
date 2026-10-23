//! A double-click on the empty canvas, driven through egui headless: the gesture its second press
//! began is dropped, so the tool is idle once the button is up.

use egui::{Event, Modifiers, PointerButton, Pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

/// One press and release at `p`.
fn click(u: &mut Ui, p: Pos2) {
    u.frame(vec![Event::PointerMoved(p)]);
    for pressed in [true, false] {
        u.frame(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    }
}

/// A double-click at `p`.
fn double_click_at(u: &mut Ui, p: Pos2) {
    u.frame(vec![Event::PointerMoved(p)]);
    for _ in 0..2 {
        for pressed in [true, false] {
            u.frame(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
        }
    }
    u.frame(vec![]);
}

#[test]
fn double_clicking_a_selected_shape_enters_vector_edit_like_enter() {
    let mut s = Session::new();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 300, "height": 200})).unwrap();
    let r: NodeId = figcraft_engine::params::id_of(&r["id"]).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let mut u = Ui::new(s);
    let p = u.app.to_screen(250.0, 200.0);
    // A click selects it; a double-click right after (three quick clicks) still edits its points.
    click(&mut u, p);
    assert_eq!(u.app.session.selection(), vec![r]);
    double_click_at(&mut u, p);
    assert!(u.app.session.ds().unwrap().vector_edit.is_some(), "a double-click enters vector edit");
    // Enter does the same.
    u.app.session.execute("vector.edit.end", &json!({})).unwrap();
    u.frame(vec![Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    assert!(u.app.session.ds().unwrap().vector_edit.is_some());
}

#[test]
fn a_slower_double_click_still_edits_the_points() {
    let mut s = Session::new();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    let r: NodeId = figcraft_engine::params::id_of(&r["id"]).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    let p = u.app.to_screen(200.0, 200.0);
    u.frame(vec![Event::PointerMoved(p)]);
    let press = |u: &mut Ui, pressed| {
        u.frame(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    };
    // The first click, then 0.4 s: inside the platform's 0.5 s double-click window, past egui's
    // default 0.3 s.
    press(&mut u, true);
    press(&mut u, false);
    for _ in 0..24 {
        u.frame(vec![]);
    }
    press(&mut u, true);
    press(&mut u, false);
    assert!(u.app.session.ds().unwrap().vector_edit.is_some(), "a double-click the platform takes edits its points");
}

#[test]
fn double_clicking_an_unselected_shape_edits_its_points() {
    let mut s = Session::new();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 300, "height": 200})).unwrap();
    let r: NodeId = figcraft_engine::params::id_of(&r["id"]).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let mut u = Ui::new(s);
    let p = u.app.to_screen(250.0, 200.0);
    // Nothing is selected: the double-click selects the shape and edits its points, like Figma.
    double_click_at(&mut u, p);
    let ve = u.app.session.ds().unwrap().vector_edit.clone();
    assert_eq!(ve.map(|v| v.id), Some(r));
}

#[test]
fn a_double_click_on_empty_canvas_right_after_entering_leaves_vector_edit() {
    let mut s = Session::new();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 300, "height": 200})).unwrap();
    let r: NodeId = figcraft_engine::params::id_of(&r["id"]).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let mut u = Ui::new(s);
    let shape = u.app.to_screen(250.0, 200.0);
    let empty = Pos2::new(u.app.canvas.rect.left() + 30.0, u.app.canvas.rect.bottom() - 30.0);
    // The double-click that enters edit mode, then straight away one on empty canvas.
    double_click_at(&mut u, shape);
    assert_eq!(u.app.session.ds().unwrap().vector_edit.as_ref().map(|v| v.id), Some(r));
    double_click_at(&mut u, empty);
    assert!(u.app.session.ds().unwrap().vector_edit.is_none(), "a double-click on empty canvas leaves vector edit at once");
}

#[test]
fn a_click_on_empty_canvas_keeps_vector_edit_until_a_double_click() {
    let mut s = Session::new();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 300, "height": 200})).unwrap();
    let r: NodeId = figcraft_engine::params::id_of(&r["id"]).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    let shape = u.app.to_screen(250.0, 200.0);
    let empty = Pos2::new(u.app.canvas.rect.left() + 30.0, u.app.canvas.rect.bottom() - 30.0);
    double_click_at(&mut u, shape);
    assert!(u.app.session.ds().unwrap().vector_edit.is_some());
    // One click on empty canvas only clears the points: edit mode stays.
    click(&mut u, empty);
    assert!(u.app.session.ds().unwrap().vector_edit.is_some(), "a click on empty canvas keeps vector edit");
    // A double-click on empty canvas leaves it, the earlier click and the counts from entering
    // included.
    double_click_at(&mut u, empty);
    assert!(u.app.session.ds().unwrap().vector_edit.is_none(), "a double-click on empty canvas leaves vector edit");
    // The next double-click edits the shape again.
    double_click_at(&mut u, shape);
    assert!(u.app.session.ds().unwrap().vector_edit.is_some(), "a double-click edits it again");
}

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
