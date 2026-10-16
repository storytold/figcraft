//! Renames in the variables window, drawn headless with the screen reader tree on: creating a
//! collection opens its name for editing, and so does a double click on its row. On every frame
//! the focused node must be in the AccessKit tree; the platform adapter (UI Automation on Windows)
//! aborts the app when it isn't (#224).

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use figcraft_engine::Session;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, text_center};

/// The variables window open on a document without collections, with AccessKit on.
fn variables_window() -> Ui {
    let mut u = Ui::new(Session::new());
    u.app.ui.variables = Some(Default::default());
    u.ctx.enable_accesskit();
    for _ in 0..10 {
        u.frame(vec![]);
    }
    u
}

/// One frame with `events`, checking the focused node is in the screen reader tree.
fn checked_frame(u: &mut Ui, events: Vec<Event>, k: usize) {
    let out = u.frame_output(events);
    let tree = out.platform_output.accesskit_update.expect("an AccessKit update");
    assert!(tree.nodes.iter().any(|(i, _)| *i == tree.focus), "frame {k}: the focused node {:?} isn't in the tree", tree.focus);
}

/// `clicks` clicks at `p` (two make a double click) and the frames that follow, each checked.
fn click_checking_focus(u: &mut Ui, p: Pos2, clicks: usize) {
    for _ in 0..10 {
        u.frame(vec![Event::PointerMoved(p)]);
    }
    let presses = [true, false].into_iter().cycle().take(clicks * 2).map(Some);
    for (k, pressed) in presses.chain(std::iter::repeat_n(None, 20)).enumerate() {
        let events = pressed.map(|pressed| Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        checked_frame(u, events.into_iter().collect(), k);
    }
}

/// Type `name` and press Enter (each frame checked).
fn type_name(u: &mut Ui, name: &str) {
    checked_frame(u, vec![Event::Text(name.into())], 0);
    for pressed in [true, false] {
        checked_frame(u, vec![Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }], 1);
    }
    for k in 0..5 {
        checked_frame(u, vec![], k + 2);
    }
}

fn collection_names(u: &Ui) -> Vec<String> {
    u.app.session.doc().unwrap().variables.collections.iter().map(|c| c.name.clone()).collect()
}

#[test]
fn creating_a_collection_puts_its_name_in_edit_with_the_focus_in_the_screen_reader_tree() {
    let mut u = variables_window();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    click_checking_focus(&mut u, text_center(&shapes, "Create collection"), 1);
    assert_eq!(collection_names(&u), ["Collection"], "a click creates one collection");
    // The keyboard is in its name: typing renames it.
    type_name(&mut u, "Brand");
    assert_eq!(collection_names(&u), ["Brand"]);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn double_clicking_a_collection_renames_it_with_the_focus_in_the_screen_reader_tree() {
    let mut u = variables_window();
    u.app.session.execute("variable.createCollection", &serde_json::json!({"name": "Spacing"})).unwrap();
    for _ in 0..10 {
        u.frame(vec![]);
    }
    let shapes = flat(&u.frame_output(vec![]).shapes);
    click_checking_focus(&mut u, text_center(&shapes, "Spacing"), 2);
    // The row's text was replaced by the field, whose text is selected: typing replaces it.
    type_name(&mut u, "Space");
    assert_eq!(collection_names(&u), ["Space"]);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}
