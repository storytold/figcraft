//! The documented shortcuts the keymap table can't check by lookup alone, driven through egui
//! headless: arrows pan with nothing selected, the keyboard box selection key and Cmd/Ctrl+arrows
//! drawing it out, list indentation while editing text, symbols another keyboard layout types with
//! Shift, the presentation keys (Space and N forward, C comments, F full screen), Alt+R for the
//! rotation origin, Enter placing a shape with its tool chosen and Cmd/Ctrl+= and - zooming the
//! canvas, not the interface.

use egui::{Event, Key, Modifiers, ViewportCommand, ViewportId};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use figcraft_engine::keymap::Platform;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn press_with(u: &mut Ui, key: Key, physical_key: Option<Key>, modifiers: Modifiers) -> egui::FullOutput {
    let ev = |pressed| Event::Key { key, physical_key, pressed, repeat: false, modifiers };
    u.frame_with(vec![ev(true), ev(false)], modifiers)
}

fn press(u: &mut Ui, key: Key, modifiers: Modifiers) -> egui::FullOutput {
    press_with(u, key, None, modifiers)
}

/// Cmd on macOS, Ctrl elsewhere, as egui reports it.
fn cmd() -> Modifiers {
    if crate::shortcuts::platform() == Platform::Mac { Modifiers::MAC_CMD | Modifiers::COMMAND } else { Modifiers::CTRL | Modifiers::COMMAND }
}

fn view(u: &Ui) -> (f64, f64, f64) {
    let v = &u.app.session.ds().unwrap().view;
    (v.cx, v.cy, v.zoom)
}

#[test]
fn arrows_pan_the_view_when_nothing_is_selected() {
    let mut s = Session::new();
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap());
    let mut u = Ui::new(s);
    u.app.run("select.set", json!({"ids": []}));
    u.app.run("view.set", json!({"zoom": 2.0, "cx": 0, "cy": 0}));
    press(&mut u, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(view(&u), (10.0, 0.0, 2.0), "20 screen pixels at 200%");
    press(&mut u, Key::ArrowUp, Modifiers::SHIFT);
    assert_eq!(view(&u), (10.0, -100.0, 2.0), "Shift: ten times as far");
    // With a selection the arrows nudge it and the view stays.
    u.app.run("select.set", json!({"ids": [r]}));
    press(&mut u, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(view(&u), (10.0, -100.0, 2.0));
    assert_eq!(u.app.session.doc().unwrap().world_bounds(r).unwrap().x0, 1.0);
}

#[test]
fn keyboard_box_selection_key_and_cmd_arrows() {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap();
    let mut u = Ui::new(s);
    // Alt+Space on macOS, Ctrl+Space on Windows and Linux (Alt+Space is the window menu there).
    let m = if crate::shortcuts::platform() == Platform::Mac { Modifiers::ALT } else { cmd() };
    press(&mut u, Key::Space, m);
    let b = u.app.ui.key_box.expect("the box shows");
    let toast = u.app.ui.toast.clone().map(|t| t.0).unwrap_or_default();
    let cmd_arrows = format!("{}+arrows", crate::toolbar::pretty_shortcut("Cmd"));
    assert!(toast.contains("Shift+arrows") && toast.contains(&cmd_arrows), "the toast names both ways to size it: {toast}");
    let zoom = view(&u).2;
    press(&mut u, Key::ArrowRight, cmd());
    let grown = u.app.ui.key_box.unwrap();
    assert_eq!((grown[0], grown[2]), (b[0], b[2] + 10.0 / zoom), "Cmd/Ctrl+arrows draw the box out");
    press(&mut u, Key::ArrowRight, cmd() | Modifiers::SHIFT);
    assert_eq!(u.app.ui.key_box.unwrap()[2], grown[2] + 50.0 / zoom, "with Shift farther");
    press(&mut u, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(u.app.ui.key_box.unwrap()[0], b[0] + 10.0 / zoom, "plain arrows move it");
    press(&mut u, Key::Escape, Modifiers::NONE);
    assert!(u.app.ui.key_box.is_none());
}

#[test]
fn cmd_brackets_indent_lists_while_editing() {
    let mut s = Session::new();
    let t = id(s.execute("text.create", &json!({"text": "One", "x": 0, "y": 0})).unwrap());
    s.execute("select.set", &json!({"ids": [t]})).unwrap();
    let mut u = Ui::new(s);
    press(&mut u, Key::Enter, Modifiers::NONE);
    press(&mut u, Key::Num8, cmd() | Modifiers::SHIFT);
    let indent = |u: &Ui| u.app.session.doc().unwrap().node(t).unwrap().kind.text().unwrap().para_list(0).indent;
    assert_eq!(indent(&u), 0);
    press(&mut u, Key::CloseBracket, cmd());
    assert_eq!(indent(&u), 1, "Cmd/Ctrl+] nests the item");
    press(&mut u, Key::OpenBracket, cmd());
    assert_eq!(indent(&u), 0, "Cmd/Ctrl+[ moves it out");
    assert!(u.app.session.ds().unwrap().text_edit.is_some(), "still editing");
}

#[test]
fn symbols_typed_with_shift_on_other_layouts_count_without_it() {
    let mut s = Session::new();
    let f = id(s.execute("node.create", &json!({"type": "frame", "x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let r = id(s.execute("node.create", &json!({"type": "rectangle", "parent": f, "x": 10, "y": 10, "width": 10, "height": 10})).unwrap());
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    // `\` typed as Shift+7 (the key's own symbol differs): Select parent, as `\` on a US keyboard.
    press_with(&mut u, Key::Backslash, Some(Key::Num7), Modifiers::SHIFT);
    assert_eq!(u.app.session.selection(), vec![f]);
    // On the key that is `\` everywhere, Shift+\ stays another chord (unbound).
    u.app.run("select.set", json!({"ids": [r]}));
    press_with(&mut u, Key::Backslash, Some(Key::Backslash), Modifiers::SHIFT);
    assert_eq!(u.app.session.selection(), vec![r]);
}

#[test]
fn presentation_keys() {
    let mut s = Session::new();
    let a = id(s.execute("node.create", &json!({"type": "frame", "name": "A", "x": 0, "y": 0, "width": 300, "height": 200})).unwrap());
    let b = id(s.execute("node.create", &json!({"type": "frame", "name": "B", "x": 400, "y": 0, "width": 300, "height": 200})).unwrap());
    let mut u = Ui::new(s);
    u.app.run("proto.present", json!({"start": a}));
    u.frame(vec![]);
    let current = |u: &Ui| u.app.inspect()["presenter"]["current"].clone();
    assert_eq!(current(&u), json!(a));
    press(&mut u, Key::N, Modifiers::NONE);
    assert_eq!(current(&u), json!(b), "N: next frame");
    press(&mut u, Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(current(&u), json!(a));
    press(&mut u, Key::Space, Modifiers::NONE);
    assert_eq!(current(&u), json!(b), "Space: next frame");
    let out = press(&mut u, Key::F, Modifiers::NONE);
    let cmds = out.viewport_output.get(&ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default();
    assert!(cmds.contains(&ViewportCommand::Fullscreen(true)), "F: full screen, {cmds:?}");
    let comments = u.app.ui.present_view.comments;
    press(&mut u, Key::C, Modifiers::NONE);
    assert_eq!(u.app.ui.present_view.comments, !comments, "C: comments");
    assert!(u.app.ui.present.is_some(), "still presenting");
}

#[test]
fn multiplayer_cursors_key_hides_and_shows_others_cursors() {
    use crate::live_tests::{Harness, cursor};
    use figcraft_engine::live::Event as LiveEvent;
    let (mut h, _) = Harness::new();
    h.frames(3);
    h.joined();
    let page = h.page();
    h.send(vec![LiveEvent::Presence(1, cursor(page, 400.0, 300.0))]);
    h.frames(2);
    assert_eq!(h.app.ui.live.shown(), vec![1], "Hana's cursor shows");
    let m =
        if crate::shortcuts::platform() == Platform::Mac { Modifiers::ALT | Modifiers::MAC_CMD | Modifiers::COMMAND } else { Modifiers::ALT | cmd() };
    let ev = |pressed| Event::Key { key: Key::Backslash, physical_key: None, pressed, repeat: false, modifiers: m };
    h.frame(vec![ev(true), ev(false)]);
    h.frames(1);
    assert!(h.app.ui.live.shown().is_empty(), "Alt+Cmd+backslash hides it");
    h.app.run("view.multiplayerCursors", json!({}));
    h.frames(2);
    assert_eq!(h.app.ui.live.shown(), vec![1], "and shows it again");
}

#[test]
fn alt_r_shows_the_rotation_origin_and_escape_hides_it() {
    let mut s = Session::new();
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap());
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    press(&mut u, Key::R, Modifiers::ALT);
    let at = u.app.session.ds().unwrap().origin_handle().expect("Alt+R shows the origin");
    assert!((at.x - 20.0).abs() < 1e-9 && (at.y - 20.0).abs() < 1e-9, "{at:?}");
    assert_eq!(u.app.session.tool.tool, figcraft_engine::ToolId::Move, "R alone would pick the Rectangle tool");
    press(&mut u, Key::Escape, Modifiers::NONE);
    assert!(u.app.session.ds().unwrap().origin_handle().is_none());
    assert_eq!(u.app.session.selection(), vec![r], "the first Escape only hides it");
    press(&mut u, Key::R, Modifiers::ALT);
    press(&mut u, Key::R, Modifiers::ALT);
    assert!(u.app.session.ds().unwrap().origin_handle().is_none(), "Alt+R again hides it too");
}

#[test]
fn enter_with_a_shape_tool_places_one_in_the_middle_of_the_view() {
    let mut u = Ui::new(Session::new());
    u.app.run("view.set", json!({"zoom": 1.0, "cx": 200, "cy": 100}));
    press(&mut u, Key::R, Modifiers::NONE);
    press(&mut u, Key::Enter, Modifiers::NONE);
    let sel = u.app.session.selection();
    assert_eq!(sel.len(), 1, "placed and selected");
    let b = u.app.session.doc().unwrap().world_bounds(sel[0]).unwrap();
    assert!((b.center().x - 200.0).abs() < 1e-9 && (b.center().y - 100.0).abs() < 1e-9 && (b.width() - 100.0).abs() < 1e-9, "{b:?}");
    // With the Move tool back, Enter on the shape edits it as before.
    press(&mut u, Key::Enter, Modifiers::NONE);
    assert!(u.app.session.ds().unwrap().vector_edit.is_some());
}

#[test]
fn full_screen_key_and_command_toggle_the_window() {
    let mut u = Ui::new(Session::new());
    let fullscreen = |out: &egui::FullOutput| -> Vec<ViewportCommand> {
        out.viewport_output.get(&ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default()
    };
    // F11 (Windows and Linux) or Ctrl+Cmd+F (macOS) asks the window to fill the screen.
    let out = if crate::shortcuts::platform() == Platform::Mac {
        press(&mut u, Key::F, Modifiers::MAC_CMD | Modifiers::COMMAND | Modifiers::CTRL)
    } else {
        press(&mut u, Key::F11, Modifiers::NONE)
    };
    assert!(fullscreen(&out).contains(&ViewportCommand::Fullscreen(true)), "full screen key, {:?}", fullscreen(&out));
    // `on: false` leaves it, whatever the window reports.
    u.app.run("view.fullscreen", json!({"on": false}));
    let out = u.frame_output(vec![]);
    assert!(fullscreen(&out).contains(&ViewportCommand::Fullscreen(false)), "{:?}", fullscreen(&out));
    // Junk parameters are ignored, not fatal.
    u.app.run("view.fullscreen", json!({"on": "yes"}));
    u.frame(vec![]);
}

#[test]
fn cmd_plus_and_minus_zoom_the_canvas_not_the_interface() {
    let mut u = Ui::new(Session::new());
    u.app.run("view.set", json!({"zoom": 1.0, "cx": 0, "cy": 0}));
    press(&mut u, Key::Equals, cmd());
    // egui applies a new zoom factor at the start of the next frame.
    u.frame(vec![]);
    assert!(view(&u).2 > 1.0, "Cmd/Ctrl+= zooms the canvas in: {}", view(&u).2);
    assert_eq!(u.ctx.zoom_factor(), 1.0, "and leaves the interface at its size");
    press(&mut u, Key::Minus, cmd());
    press(&mut u, Key::Minus, cmd());
    u.frame(vec![]);
    assert!(view(&u).2 < 1.0, "Cmd/Ctrl+- zooms the canvas out: {}", view(&u).2);
    assert_eq!(u.ctx.zoom_factor(), 1.0);
}
