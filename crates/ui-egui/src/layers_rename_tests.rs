//! Renaming layers in the layers panel, driven through egui headless: a double-click or Cmd/Ctrl+R
//! starts it, Tab / Shift+Tab commit and go on to the next / previous row (the selection follows),
//! Enter commits and Escape cancels.

use egui::{Event, Key, Modifiers};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn press(u: &mut Ui, key: Key, modifiers: Modifiers) {
    u.frame(vec![
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
        Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
    ]);
}

fn type_text(u: &mut Ui, text: &str) {
    u.frame(vec![Event::Text(text.into())]);
}

fn name(u: &Ui, id: NodeId) -> String {
    u.app.session.doc().unwrap().node(id).unwrap().name.clone()
}

fn renaming(u: &Ui) -> Option<NodeId> {
    u.app.ui.rename.map(|r| r.id)
}

/// Rows top to bottom: `c` (rectangle), `frame` (collapsed, holding `child`), `a` (rectangle).
fn setup() -> (Ui, NodeId, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    let a = id(s.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let frame = id(s.execute("node.create", &json!({"type": "frame", "x": 50, "y": 0, "width": 100, "height": 100})).unwrap());
    let child = id(s.execute("node.create", &json!({"type": "ellipse", "parent": frame, "x": 10, "y": 10, "width": 10, "height": 10})).unwrap());
    let c = id(s.execute("node.create", &json!({"type": "rectangle", "x": 200, "y": 0, "width": 10, "height": 10})).unwrap());
    s.execute("select.none", &json!({})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.expanded.clear();
    u.frame(vec![]);
    (u, a, frame, child, c)
}

/// Double-click `id`'s row and let its name field take the keyboard.
fn double_click(u: &mut Ui, id: NodeId) {
    let p = u.row_at(id);
    for _ in 0..2 {
        u.button(p, true);
        u.button(p, false);
    }
    u.frame(vec![]);
    assert_eq!(renaming(u), Some(id));
    assert!(u.ctx.memory(|m| m.focused().is_some()), "the name field has the keyboard");
}

#[test]
fn tab_and_shift_tab_commit_and_rename_the_next_and_previous_rows() {
    let (mut u, _, frame, _, c) = setup();
    double_click(&mut u, c);
    type_text(&mut u, "Top");
    press(&mut u, Key::Tab, Modifiers::NONE);
    assert_eq!(name(&u, c), "Top");
    assert_eq!(renaming(&u), Some(frame), "Tab renames the row below");
    assert_eq!(u.app.session.selection(), vec![frame], "the selection follows the rename");
    u.frame(vec![]);
    // The whole name is selected: typing replaces it.
    type_text(&mut u, "Box");
    press(&mut u, Key::Tab, Modifiers::SHIFT);
    assert_eq!(name(&u, frame), "Box");
    assert_eq!(renaming(&u), Some(c), "Shift+Tab renames the row above");
    assert_eq!(u.app.session.selection(), vec![c]);
    u.frame(vec![]);
    type_text(&mut u, "Discarded");
    press(&mut u, Key::Escape, Modifiers::NONE);
    assert_eq!(renaming(&u), None, "Escape ends the rename");
    assert_eq!(name(&u, c), "Top", "Escape keeps the old name");
    assert_eq!(u.app.session.selection(), vec![c], "Escape only ends the rename");
    // A later rename of the same row starts from its name again.
    double_click(&mut u, c);
    press(&mut u, Key::Enter, Modifiers::NONE);
    assert_eq!(renaming(&u), None, "Enter ends the rename");
    assert_eq!(name(&u, c), "Top");
}

#[test]
fn tab_on_the_last_row_commits_and_ends() {
    let (mut u, a, _, _, _) = setup();
    double_click(&mut u, a);
    type_text(&mut u, "Bottom");
    press(&mut u, Key::Tab, Modifiers::NONE);
    assert_eq!(name(&u, a), "Bottom");
    assert_eq!(renaming(&u), None);
    assert_eq!(u.app.session.selection(), vec![a]);
}

#[test]
fn cmd_r_renames_the_selection_and_opens_its_parents() {
    // Cmd+R on macOS; Ctrl+R elsewhere, where egui sets both `ctrl` and `command`. On macOS Ctrl
    // comes without `command` and Ctrl+R isn't a shortcut.
    let cmd = Modifiers::MAC_CMD | Modifiers::COMMAND;
    let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
    let keys: &[Modifiers] = if cfg!(target_os = "macos") { &[cmd] } else { &[cmd, ctrl] };
    for (k, &held) in keys.iter().enumerate() {
        let (mut u, _, frame, child, _) = setup();
        u.app.session.execute("select.set", &json!({"ids": [child]})).unwrap();
        u.frame(vec![]);
        // Collapsed again after the selection was revealed.
        u.app.ui.expanded.clear();
        u.frame(vec![]);
        press(&mut u, Key::R, held);
        assert!(u.app.ui.expanded.contains(&frame), "the renamed layer's parent opens ({k})");
        assert_eq!(renaming(&u), Some(child), "({k})");
        u.frame(vec![]);
        type_text(&mut u, "Dot");
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert_eq!(name(&u, child), "Dot", "({k})");
        assert_eq!(renaming(&u), None);
    }
}

#[test]
fn rename_without_a_name_renames_one_row_or_opens_the_dialog() {
    let (mut u, a, _, _, c) = setup();
    let before = name(&u, a);
    u.app.run("object.rename", json!({"ids": [c]}));
    assert_eq!(renaming(&u), Some(c), "one layer renames in its row");
    assert_eq!(u.app.session.selection(), vec![c]);
    u.app.ui.rename = None;
    // Several layers: the Rename layers dialog instead (layers_behaviour_tests).
    u.app.run("object.rename", json!({"ids": [a, c]}));
    assert_eq!(renaming(&u), None);
    assert!(u.app.ui.batch_rename.is_some(), "several layers open the Rename layers dialog");
    assert_eq!(name(&u, a), before, "nothing renamed yet");
    // Headless, the command changes nothing and leaves no undo step.
    let mut s = Session::new();
    let r = id(s.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let doc = s.ds().unwrap().doc.clone();
    s.execute("object.rename", &json!({"ids": [r]})).unwrap();
    assert!(std::sync::Arc::ptr_eq(&doc, &s.ds().unwrap().doc));
}

/// Asserts a layer rename continues after its row is scrolled out of view and commits on
/// Enter.
#[test]
fn a_rename_scrolled_out_of_view_goes_on() {
    let mut s = Session::new();
    let ids: Vec<NodeId> = (0..60)
        .map(|k| id(s.execute("node.create", &json!({"type": "rectangle", "x": k * 20, "y": 0, "width": 10, "height": 10})).unwrap()))
        .collect();
    s.execute("select.none", &json!({})).unwrap();
    let mut u = Ui::new(s);
    // The top row (the last created) is listed first; scroll it away while renaming it.
    let top = *ids.last().unwrap();
    double_click(&mut u, top);
    let p = egui::pos2(150.0, 400.0);
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        source: egui::MouseWheelSource::Unknown,
        delta: egui::vec2(0.0, -400.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    };
    for _ in 0..10 {
        u.frame(vec![Event::PointerMoved(p), wheel.clone()]);
    }
    type_text(&mut u, "Far");
    assert_eq!(renaming(&u), Some(top), "still renaming");
    press(&mut u, Key::Enter, Modifiers::NONE);
    assert_eq!(name(&u, top), "Far");
}

#[test]
fn a_key_typed_in_the_frame_a_rename_starts_replaces_the_name() {
    // The app draws no frame between the double-click's last release (or F2, or File ▸ Rename) and
    // the first key, so the name field first shows in the frame that types into it.
    let (mut u, a, _, _, c) = setup();
    let p = u.row_at(c);
    for _ in 0..2 {
        u.button(p, true);
        u.button(p, false);
    }
    type_text(&mut u, "Hero");
    press(&mut u, Key::Enter, Modifiers::NONE);
    assert_eq!(name(&u, c), "Hero", "a double-click rename");
    u.app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
    u.frame(vec![]);
    press(&mut u, Key::F2, Modifiers::NONE);
    type_text(&mut u, "Base");
    press(&mut u, Key::Enter, Modifiers::NONE);
    assert_eq!(name(&u, a), "Base", "an F2 rename");
    crate::menus::invoke(&mut u.app, "file.rename");
    type_text(&mut u, "Brand kit");
    press(&mut u, Key::Enter, Modifiers::NONE);
    assert_eq!(u.app.session.doc().unwrap().name, "Brand kit", "File ▸ Rename");
}
