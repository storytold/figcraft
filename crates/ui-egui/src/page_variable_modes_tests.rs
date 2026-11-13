//! The Design panel's Variable modes section for the page, driven headless: with nothing
//! selected a collection with several modes lists them, as Figma's Page section does.

use egui::{Event, Modifiers, PointerButton, Pos2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, text_center};

/// A click at `p`, and the frames that follow; what the last frame drew.
fn click(u: &mut Ui, p: Pos2) -> Vec<egui::Shape> {
    for _ in 0..5 {
        u.frame(vec![Event::PointerMoved(p)]);
    }
    let mut shapes = Vec::new();
    for pressed in [Some(true), Some(false)].into_iter().chain(std::iter::repeat_n(None, 20)) {
        let events = pressed.map(|pressed| Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        shapes = flat(&u.frame_output(events.into_iter().collect()).shapes);
    }
    shapes
}

fn scene(modes: &[&str]) -> Ui {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": modes})).unwrap();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    Ui::new(s)
}

#[test]
fn with_nothing_selected_the_page_shows_its_variable_modes() {
    let mut u = scene(&["Light", "Dark"]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(crate::live_tests::texts(&shapes).iter().any(|(t, ..)| t == crate::i18n::tr("Variable modes")), "the section shows");
    let history = u.app.session.undo_depth().0;
    let menu = click(&mut u, text_center(&shapes, "Auto (Light)"));
    click(&mut u, text_center(&menu, "    Dark"));
    let page = u.app.session.page().unwrap();
    let doc = u.app.session.doc().unwrap();
    let coll = doc.variables.collections[0].clone();
    let dark = coll.modes.iter().find(|m| m.name == "Dark").unwrap().id.clone();
    assert_eq!(doc.node(page).unwrap().var_modes.get(&coll.id), Some(&dark));
    assert_eq!(u.app.session.undo_depth().0, history + 1, "one undo step");
}

#[test]
fn the_page_section_has_no_variable_modes_without_a_multi_mode_collection() {
    let mut u = scene(&["Only"]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(!crate::live_tests::texts(&shapes).iter().any(|(t, ..)| t == crate::i18n::tr("Variable modes")));
}
