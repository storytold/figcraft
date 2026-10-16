//! Renaming a folder of local styles, drawn headless with the screen reader tree on: Rename folder
//! in the folder's menu opens its name for editing. On every frame the focused node must be in the
//! AccessKit tree; the platform adapter (UI Automation on Windows) aborts the app when it isn't
//! (the same fault as #224).

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, text_center};

/// One frame with `events`, checking the focused node is in the screen reader tree.
fn checked_frame(u: &mut Ui, events: Vec<Event>, k: usize) -> Vec<egui::Shape> {
    let out = u.frame_output(events);
    let tree = out.platform_output.accesskit_update.expect("an AccessKit update");
    assert!(tree.nodes.iter().any(|(i, _)| *i == tree.focus), "frame {k}: the focused node {:?} isn't in the tree", tree.focus);
    flat(&out.shapes)
}

/// A click with `button` at `p` and the frames that follow, each checked.
fn click(u: &mut Ui, p: Pos2, button: PointerButton) -> Vec<egui::Shape> {
    for _ in 0..5 {
        u.frame(vec![Event::PointerMoved(p)]);
    }
    let mut shapes = Vec::new();
    for (k, pressed) in [Some(true), Some(false)].into_iter().chain(std::iter::repeat_n(None, 20)).enumerate() {
        let events = pressed.map(|pressed| Event::PointerButton { pos: p, button, pressed, modifiers: Modifiers::NONE });
        shapes = checked_frame(u, events.into_iter().collect(), k);
    }
    shapes
}

#[test]
fn renaming_a_style_folder_keeps_the_focus_in_the_screen_reader_tree() {
    let mut s = Session::new();
    for name in ["Brand/Primary", "Brand/Accent"] {
        s.execute("style.create", &json!({"type": "paint", "name": name, "value": [{"type": "solid", "color": "#0D99FF"}]})).unwrap();
    }
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    let shapes = checked_frame(&mut u, vec![], 0);
    // The Design panel lists the local styles with nothing selected; the folder's menu renames it.
    let menu = click(&mut u, text_center(&shapes, "Brand"), PointerButton::Secondary);
    click(&mut u, text_center(&menu, "Rename folder"), PointerButton::Primary);
    checked_frame(&mut u, vec![Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND }], 0);
    checked_frame(&mut u, vec![Event::Text("Theme".into())], 1);
    for pressed in [true, false] {
        checked_frame(&mut u, vec![Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }], 2);
    }
    for k in 0..5 {
        checked_frame(&mut u, vec![], k + 3);
    }
    let names: Vec<String> = u.app.session.doc().unwrap().styles.iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, ["Theme/Primary", "Theme/Accent"]);
}
