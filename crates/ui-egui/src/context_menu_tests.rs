//! The right-click menu on a layer: the copy commands sit in a Copy as submenu under Paste to
//! replace; "Copy link to selection" puts `figcraft://node/<id>` on the clipboard and says so, and
//! with nothing selected the command copies nothing.

use std::sync::{Arc, Mutex};

use egui::Pos2;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center};
use crate::present_tests::id;

/// A clipboard that keeps what was copied where the test can read it.
struct Copied(Arc<Mutex<Vec<String>>>);

impl crate::Services for Copied {
    fn clipboard_set_text(&mut self, s: &str) {
        if let Ok(mut copied) = self.0.lock() {
            copied.push(s.to_string());
        }
    }
}

#[test]
fn the_context_menu_groups_the_copy_commands_under_copy_as() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 200, "name": "หน้าแรก"})).unwrap());
    let copied = Arc::new(Mutex::new(Vec::new()));
    let mut u = Ui::new(s);
    u.app.services = Box::new(Copied(copied.clone()));
    u.frame(vec![]);
    u.app.ui.context_menu = Some((Pos2::new(700.0, 300.0), vec![f]));
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    // A row under Paste to replace; the items are in its submenu, not in the menu itself.
    let y = |s: &str| text_center(&shapes, s).y;
    assert!(y("Paste to replace") < y("Copy as") && y("Copy as") < y("Bring to front"), "Copy as sits under Paste to replace");
    for item in ["Copy as CSS", "Copy as SVG", "Copy as PNG", "Copy link to selection"] {
        assert!(!has_text(&shapes, item), "{item} is inside the submenu");
    }
    let at = text_center(&shapes, "Copy as");
    u.button(at, true);
    u.button(at, false);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    for item in ["Copy as CSS", "Copy as SVG", "Copy as PNG", "Copy link to selection"] {
        assert!(has_text(&shapes, item), "{item} is in the submenu");
    }
    let at = text_center(&shapes, "Copy link to selection");
    u.button(at, true);
    u.button(at, false);
    assert_eq!(*copied.lock().unwrap(), vec![format!("figcraft://node/{}", f.0)], "the link to the frame");
    assert_eq!(u.app.ui.toast.as_ref().map(|t| t.0.as_str()), Some("Link copied"));
    assert!(u.app.ui.context_menu.is_none(), "the menu closes");
}

#[test]
fn the_context_menu_offers_go_to_main_for_an_instance() {
    // Right-clicking an instance reaches its main component: the menu offers "Go to main
    // component" instead of "Create component" (which is for plain layers).
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 60, "name": "Button"})).unwrap());
    let comp = id(&s.execute("component.create", &json!({"ids": [f]})).unwrap());
    let inst = id(&s.execute("instance.create", &json!({"component": comp, "x": 0, "y": 200})).unwrap());
    let mut u = Ui::new(s);
    u.frame(vec![]);
    u.app.ui.context_menu = Some((Pos2::new(700.0, 300.0), vec![inst]));
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&shapes, "Go to main component"), "an instance's menu reaches its main");
    assert!(!has_text(&shapes, "Create component"), "not offered for an instance");
}

#[test]
fn copying_a_link_with_nothing_selected_copies_nothing() {
    let mut s = Session::new();
    s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 200})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let copied = Arc::new(Mutex::new(Vec::new()));
    let mut u = Ui::new(s);
    u.app.services = Box::new(Copied(copied.clone()));
    crate::menus::invoke(&mut u.app, "edit.copyLink");
    assert!(copied.lock().unwrap().is_empty(), "no link to node 0");
    assert!(u.app.ui.toast.is_none());
}

/// With a section selected, a right-click on either frame inside it links that frame, not the
/// section that was selected before.
#[test]
fn copying_a_link_on_a_frame_inside_a_selected_section_links_that_frame() {
    let mut s = Session::new();
    let sec = id(&s.execute("node.create", &json!({"type": "section", "x": 0, "y": 0, "width": 800, "height": 400, "name": "Flow"})).unwrap());
    let a = id(&s.execute("frame.create", &json!({"x": 20, "y": 60, "width": 300, "height": 200, "name": "main", "parent": sec})).unwrap());
    let b = id(&s.execute("frame.create", &json!({"x": 400, "y": 60, "width": 300, "height": 200, "name": "dropdown", "parent": sec})).unwrap());
    s.set_selection(vec![sec]);
    let copied = Arc::new(Mutex::new(Vec::new()));
    let mut u = Ui::new(s);
    u.app.services = Box::new(Copied(copied.clone()));
    u.frame(vec![]);
    for (frame, x) in [(a, 170.0), (b, 550.0)] {
        let at = u.app.to_screen(x, 160.0);
        crate::canvas::open_context_menu(&mut u.app, at, false);
        crate::menus::invoke(&mut u.app, "edit.copyLink");
        let last = copied.lock().unwrap().last().cloned();
        assert_eq!(last, Some(format!("figcraft://node/{}", frame.0)), "the frame at x={x}");
    }
}

/// A frame inside a section has its label above it, inside the section: a right-click there picks
/// the frame whose label it is, not the section behind the label.
#[test]
fn copying_a_link_from_a_frames_name_label_links_that_frame() {
    let mut s = Session::new();
    let sec = id(&s.execute("node.create", &json!({"type": "section", "x": 0, "y": 0, "width": 800, "height": 400, "name": "Flow"})).unwrap());
    let a = id(&s.execute("frame.create", &json!({"x": 20, "y": 80, "width": 300, "height": 200, "name": "main", "parent": sec})).unwrap());
    s.set_selection(vec![sec]);
    let copied = Arc::new(Mutex::new(Vec::new()));
    let mut u = Ui::new(s);
    u.app.services = Box::new(Copied(copied.clone()));
    u.frame(vec![]);
    let quad = u.app.session.tool.labels.iter().find(|(i, _)| *i == a).map(|(_, q)| *q).expect("the frame has a label");
    let (cx, cy) = (quad.iter().map(|p| p.x).sum::<f64>() / 4.0, quad.iter().map(|p| p.y).sum::<f64>() / 4.0);
    let at = u.app.to_screen(cx, cy);
    crate::canvas::open_context_menu(&mut u.app, at, false);
    assert_eq!(u.app.session.selection(), vec![a], "the label's frame is selected");
    crate::menus::invoke(&mut u.app, "edit.copyLink");
    assert_eq!(copied.lock().unwrap().last().cloned(), Some(format!("figcraft://node/{}", a.0)));
}

/// Two overlapping rectangles, the upper one locked; the menu opens on the overlap with nothing
/// selected. Returns the app's UI and the ids (lower, locked).
fn overlap_ui() -> (Ui, figcraft_doc::NodeId, figcraft_doc::NodeId) {
    let mut s = Session::new();
    let lower = id(&s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 80, "height": 80, "name": "Lower"})).unwrap());
    let upper = id(&s.execute("shape.rectangle", &json!({"x": 120, "y": 120, "width": 80, "height": 80, "name": "Upper"})).unwrap());
    s.execute("object.toggleLock", &json!({"ids": [upper], "locked": true})).unwrap();
    let mut u = Ui::new(s);
    u.frame(vec![]);
    (u, lower, upper)
}

/// Measured in Figma's web app, 2026-11-10 (Lock and unlock layers: locked layers are listed in Select layer).
#[test]
fn select_layer_marks_locked_layers_with_a_padlock_and_picks_them() {
    let (mut u, lower, upper) = overlap_ui();
    let at = u.app.to_screen(150.0, 150.0);
    crate::canvas::open_context_menu(&mut u.app, at, false);
    u.app.session.set_selection(vec![]);
    crate::menus::open_select_layer(&u.ctx, true);
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    // The Layers panel lists the same names; the menu's rows are the right-most copies.
    let menu_row = |name: &str| {
        let (_, pos, size) = crate::live_tests::texts(&shapes)
            .into_iter()
            .filter(|(t, _, _)| t == name)
            .max_by(|a, b| a.1.x.total_cmp(&b.1.x))
            .unwrap_or_else(|| panic!("no row {name:?}"));
        (pos, size)
    };
    let (lower_at, upper_at) = (menu_row("Lower"), menu_row("Upper"));
    assert!(lower_at.0.x > 300.0 && upper_at.0.x > 300.0, "both layers are listed in the menu");
    let in_leading = |(pos, size): (Pos2, egui::Vec2)| {
        let c = pos + size / 2.0;
        shapes
            .iter()
            .filter(|s| !matches!(s, egui::Shape::Text(_)))
            .filter(|s| {
                let r = s.visual_bounding_rect();
                r.is_positive()
                    && (r.center().y - c.y).abs() < 8.0
                    && r.max.x < c.x - size.x / 2.0 - 4.0
                    && r.min.x > c.x - size.x / 2.0 - 40.0
                    && r.width() < 20.0
            })
            .count()
    };
    assert!(in_leading(upper_at) > 0, "the locked row has a padlock in its leading column");
    assert_eq!(in_leading(lower_at), 0, "the other row has nothing there");
    let p = upper_at.0 + upper_at.1 / 2.0;
    for pressed in [true, false] {
        u.frame(vec![
            egui::Event::PointerMoved(p),
            egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE },
        ]);
    }
    assert_eq!(u.app.session.selection(), vec![upper], "picking the locked row selects it");
    let _ = lower;
}

/// Measured on a single locked layer only (Figma's web app, 2026-11-10): the least certain rule.
#[test]
fn select_layer_needs_more_than_one_layer_under_the_pointer() {
    let (mut u, _, _) = overlap_ui();
    let one = u.app.to_screen(105.0, 105.0);
    crate::canvas::open_context_menu(&mut u.app, one, false);
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(!has_text(&shapes, "Select layer"), "one layer under the pointer: no Select layer");
    let two = u.app.to_screen(150.0, 150.0);
    crate::canvas::open_context_menu(&mut u.app, two, false);
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&shapes, "Select layer"), "two layers: it shows");
}
