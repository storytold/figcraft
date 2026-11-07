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
