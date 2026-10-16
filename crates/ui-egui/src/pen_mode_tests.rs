//! Leaving Design with a pen path open (#307): the path ends as Escape ends it, the tool goes back
//! to Move, and the canvas of Prototype or Inspect takes no more pen points. Driven by the panel
//! keys and by the toolbar's mode switcher.

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, Pos2, pos2};
use figcraft_engine::Session;
use figcraft_engine::tools::ToolId;
use serde_json::json;

use crate::RightTab;
use crate::layers_drag_tests::Ui;

fn press(u: &mut Ui, key: Key, modifiers: Modifiers) {
    u.frame(vec![
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
        Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
    ]);
}

fn click(u: &mut Ui, p: Pos2) {
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![]);
}

fn layers(u: &Ui) -> usize {
    let doc = u.app.session.doc().unwrap();
    doc.node(u.app.session.page().unwrap()).unwrap().children.len()
}

/// The middle of the toolbar's mode button named `label`, found in the AccessKit tree.
fn mode_button(u: &mut Ui, label: &str) -> Pos2 {
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let n = nodes.iter().map(|(_, n)| n).find(|n| n.role() == Role::Button && n.label() == Some(label)).expect(label);
    let b = n.bounds().expect("bounds");
    pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

/// A pen path with two anchors still open on an empty page.
fn drawing() -> Ui {
    let mut u = Ui::new(Session::new());
    u.app.run("tool.set", json!({"tool": "pen"}));
    click(&mut u, pos2(600.0, 400.0));
    click(&mut u, pos2(700.0, 450.0));
    assert_eq!(u.app.session.tool.pen.as_ref().map(|p| p.points.len()), Some(2), "the path is open");
    u
}

#[test]
fn switching_mode_ends_the_pen_path_as_escape_does() {
    let mut esc = drawing();
    press(&mut esc, Key::Escape, Modifiers::NONE);
    assert!(esc.app.session.tool.pen.is_none());
    let finished = layers(&esc);
    assert_eq!(finished, 1, "Escape keeps the two-anchor path");

    // Shift+E is Prototype, Shift+D is Inspect.
    for key in [Key::E, Key::D] {
        let mut u = drawing();
        press(&mut u, key, Modifiers::SHIFT);
        assert_ne!(u.app.ui.right_tab, RightTab::Design, "{key:?}");
        assert!(u.app.session.tool.pen.is_none(), "{key:?}: the path is finished");
        assert_eq!(u.app.session.tool.tool, ToolId::Move, "{key:?}: back to Move");
        assert_eq!(layers(&u), finished, "{key:?}: the path is kept as Escape keeps it");
        // The canvas takes no more pen points.
        click(&mut u, pos2(800.0, 300.0));
        assert!(u.app.session.tool.pen.is_none(), "{key:?}: a click starts no path");
        assert_eq!(layers(&u), finished, "{key:?}: a click adds nothing");
    }
}

#[test]
fn the_mode_switcher_ends_the_pen_path_too() {
    let finished = {
        let mut esc = drawing();
        press(&mut esc, Key::Escape, Modifiers::NONE);
        layers(&esc)
    };
    for (label, tab) in [("Prototype", RightTab::Prototype), ("Inspect", RightTab::Inspect)] {
        let mut u = drawing();
        let at = mode_button(&mut u, label);
        click(&mut u, at);
        assert_eq!(u.app.ui.right_tab, tab, "{label}");
        assert!(u.app.session.tool.pen.is_none(), "{label}: the path is finished");
        assert_eq!(u.app.session.tool.tool, ToolId::Move, "{label}: back to Move");
        assert_eq!(layers(&u), finished, "{label}: the path is kept as Escape keeps it");
    }
    // Design is already the mode while the pen draws: pressing it again leaves nothing, so the
    // path goes on.
    let mut u = drawing();
    let at = mode_button(&mut u, "Design");
    click(&mut u, at);
    assert_eq!(u.app.ui.right_tab, RightTab::Design);
    assert!(u.app.session.tool.pen.is_some(), "Design: the path goes on");
    assert_eq!(u.app.session.tool.tool, ToolId::Pen, "Design: still the pen");
}

#[test]
fn a_press_on_the_mode_switcher_does_not_reach_the_canvas() {
    // The switcher floats over the canvas: the pen must not place an anchor under it.
    for label in ["Prototype", "Design"] {
        let mut u = Ui::new(Session::new());
        u.app.run("tool.set", json!({"tool": "pen"}));
        let at = mode_button(&mut u, label);
        u.button(at, true);
        assert!(u.app.session.tool.pen.is_none(), "{label}: the press starts no path");
        u.button(at, false);
        u.frame(vec![]);
        assert!(u.app.session.tool.pen.is_none(), "{label}: no path after the click");
        assert_eq!(layers(&u), 0, "{label}: the click adds nothing");
    }
}
