//! Keyboard shortcuts driven through egui headless: number keys for opacity (two quick digits make
//! an exact percentage), Shift+0/1/2 zoom, alignment and Shift+E/Shift+D panel keys, Cmd/Ctrl+K on
//! the canvas and while editing text, text formatting keys while editing, and the hold-Z zoom tool.
//! Modifiers are Cmd/Alt/Shift only, which mean the same on every platform.

use egui::{Event, Key, Modifiers, pos2};
use figcraft_doc::{NodeId, TextDecoration};
use figcraft_engine::Session;
use figcraft_engine::keymap::KEYS;
use serde_json::json;

use crate::RightTab;
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

fn opacity(u: &Ui, id: NodeId) -> f32 {
    u.app.session.doc().unwrap().node(id).unwrap().opacity
}

fn zoom(u: &Ui) -> f64 {
    u.app.session.ds().unwrap().view.zoom
}

/// Longer than the window for a second opacity digit.
fn pause(u: &mut Ui) {
    for _ in 0..60 {
        u.frame(vec![]);
    }
}

/// Two rectangles, both selected.
fn setup() -> (Ui, NodeId, NodeId) {
    let mut s = Session::new();
    let a = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap());
    let b = id(s.execute("shape.rectangle", &json!({"x": 100, "y": 30, "width": 20, "height": 20})).unwrap());
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    (Ui::new(s), a, b)
}

#[test]
fn every_shortcut_reaches_something_the_app_runs() {
    for b in KEYS {
        assert!(crate::menus::available(b.id), "{} is bound but nothing runs it", b.id);
    }
}

#[test]
fn number_keys_set_opacity_and_two_quick_digits_an_exact_percentage() {
    let (mut u, a, b) = setup();
    press(&mut u, Key::Num5, Modifiers::NONE);
    assert_eq!((opacity(&u, a), opacity(&u, b)), (0.5, 0.5));
    pause(&mut u);
    // 4 then 5 straight after: 45%.
    press(&mut u, Key::Num4, Modifiers::NONE);
    assert_eq!(opacity(&u, a), 0.4);
    press(&mut u, Key::Num5, Modifiers::NONE);
    assert_eq!(opacity(&u, a), 0.45);
    // A pause starts over: 0 alone is 100%.
    pause(&mut u);
    press(&mut u, Key::Num0, Modifiers::NONE);
    assert_eq!(opacity(&u, a), 1.0);
    // Off in Preferences: digits do nothing.
    u.app.run("pref.numberKeysOpacity", json!({"on": false}));
    pause(&mut u);
    press(&mut u, Key::Num3, Modifiers::NONE);
    assert_eq!(opacity(&u, a), 1.0);
}

#[test]
fn shift_digits_zoom() {
    let (mut u, _, b) = setup();
    u.app.run("view.set", json!({"zoom": 0.5}));
    press(&mut u, Key::Num0, Modifiers::SHIFT);
    assert_eq!(zoom(&u), 1.0, "Shift+0: 100%");
    // Shift+1 arrives as `!` on most layouts.
    press(&mut u, Key::Exclamationmark, Modifiers::SHIFT);
    let fit = zoom(&u);
    assert!(fit > 1.0, "Shift+1 fits the two small rectangles: {fit}");
    u.app.run("select.set", json!({"ids": [b]}));
    press(&mut u, Key::Num2, Modifiers::SHIFT);
    assert!(zoom(&u) > fit, "Shift+2 fits the smaller selection");
    let v = &u.app.session.ds().unwrap().view;
    assert_eq!((v.cx, v.cy), (110.0, 40.0), "centred on it");
    // `+` and `-` step the zoom.
    let z = zoom(&u);
    press(&mut u, Key::Minus, Modifiers::NONE);
    let out = zoom(&u);
    assert!(out < z, "- zooms out");
    press(&mut u, Key::Plus, Modifiers::SHIFT);
    assert!(zoom(&u) > out, "+ zooms in");
}

#[test]
fn alignment_keys() {
    let (mut u, a, b) = setup();
    let x = |u: &Ui, id: NodeId| u.app.session.doc().unwrap().world_bounds(id).unwrap();
    press(&mut u, Key::D, Modifiers::ALT);
    assert_eq!((x(&u, a).x1, x(&u, b).x1), (120.0, 120.0), "Alt+D aligns right edges");
    press(&mut u, Key::W, Modifiers::ALT);
    assert_eq!((x(&u, a).y0, x(&u, b).y0), (0.0, 0.0), "Alt+W aligns tops");
    press(&mut u, Key::V, Modifiers::ALT);
    assert_eq!((x(&u, a).center().y, x(&u, b).center().y), (20.0, 20.0), "Alt+V centres vertically");
}

#[test]
fn panel_keys_and_cmd_k() {
    let (mut u, _, _) = setup();
    press(&mut u, Key::E, Modifiers::SHIFT);
    assert_eq!(u.app.ui.right_tab, RightTab::Prototype, "Shift+E: Prototype");
    press(&mut u, Key::E, Modifiers::SHIFT);
    assert_eq!(u.app.ui.right_tab, RightTab::Design, "and back to Design");
    press(&mut u, Key::D, Modifiers::SHIFT);
    assert_eq!(u.app.ui.right_tab, RightTab::Inspect, "Shift+D: Dev Mode");
    press(&mut u, Key::D, Modifiers::SHIFT);
    assert_eq!(u.app.ui.right_tab, RightTab::Design);
    let collapsed = u.app.ui.left_collapsed;
    press(&mut u, Key::Backslash, Modifiers::SHIFT | Modifiers::COMMAND);
    assert_eq!(u.app.ui.left_collapsed, !collapsed, "Shift+Cmd+\\ toggles the left sidebar");
    press(&mut u, Key::K, Modifiers::COMMAND);
    assert!(u.app.ui.palette.is_some(), "Cmd+K opens Actions on the canvas");
}

#[test]
fn text_keys_while_editing() {
    let mut s = Session::new();
    let t = id(s.execute("text.create", &json!({"text": "Hello", "x": 0, "y": 0})).unwrap());
    s.execute("select.set", &json!({"ids": [t]})).unwrap();
    let mut u = Ui::new(s);
    press(&mut u, Key::Enter, Modifiers::NONE);
    assert!(u.app.session.ds().unwrap().text_edit.is_some(), "Enter edits the text");
    press(&mut u, Key::A, Modifiers::COMMAND);
    press(&mut u, Key::U, Modifiers::COMMAND);
    let style = |u: &Ui| u.app.session.doc().unwrap().node(t).unwrap().kind.text().unwrap().style_at(1).clone();
    assert_eq!(style(&u).decoration, TextDecoration::Underline, "Cmd+U underlines the selection");
    let size = style(&u).font_size;
    press(&mut u, Key::Period, Modifiers::SHIFT | Modifiers::COMMAND);
    assert!(style(&u).font_size > size, "Cmd+Shift+> grows the text");
    // Plain letters type; Cmd+K doesn't open Actions while editing.
    press(&mut u, Key::K, Modifiers::COMMAND);
    assert!(u.app.ui.palette.is_none());
    assert!(u.app.session.ds().unwrap().text_edit.is_some(), "still editing");
}

#[test]
fn hold_z_zoom_tool() {
    let (mut u, _, _) = setup();
    u.app.run("view.set", json!({"zoom": 1.0}));
    let p = pos2(700.0, 450.0);
    let held = Modifiers::NONE;
    u.frame(vec![Event::PointerMoved(p), Event::Key { key: Key::Z, physical_key: None, pressed: true, repeat: false, modifiers: held }]);
    let before = u.app.to_page(p);
    u.button(p, true);
    u.button(p, false);
    assert_eq!(zoom(&u), 2.0, "a click with Z held zooms in a step");
    let after = u.app.to_page(p);
    assert!((before.x - after.x).abs() < 0.01 && (before.y - after.y).abs() < 0.01, "about the pointer");
    assert_eq!(u.app.session.selection().len(), 2, "the click didn't reach the move tool");
    u.frame(vec![Event::Key { key: Key::Z, physical_key: None, pressed: false, repeat: false, modifiers: held }]);
    u.button(p, true);
    u.button(p, false);
    assert_eq!(zoom(&u), 2.0, "released: clicks select again");
}

#[test]
fn shortcuts_panel_opens_switches_tabs_and_searches() {
    let (mut u, _, _) = setup();
    // Ctrl+Shift+? (Control on macOS too).
    press(&mut u, Key::Questionmark, Modifiers { ctrl: true, command: !cfg!(target_os = "macos"), shift: true, ..Modifiers::NONE });
    assert!(u.app.ui.shortcuts_panel, "Ctrl+Shift+? opens the panel");
    for g in figcraft_engine::keymap::Group::ALL {
        u.app.ui.shortcuts_tab = g;
        u.frame(vec![]);
    }
    u.app.ui.shortcuts_query = "zoom".into();
    u.frame(vec![]);
    u.app.ui.shortcuts_query = "no such shortcut".into();
    u.frame(vec![]);
    u.app.run("help.shortcuts", json!({}));
    assert!(!u.app.ui.shortcuts_panel, "the command toggles it");
}

#[test]
fn shortcuts_panel_does_not_show_on_the_home_screen() {
    let mut u = Ui::new(Session::empty());
    u.app.ui.home = true;
    u.app.ui.shortcuts_panel = true;
    u.frame(vec![]);
    assert!(!u.app.ui.shortcuts_panel, "the panel closes on the home screen");
}
