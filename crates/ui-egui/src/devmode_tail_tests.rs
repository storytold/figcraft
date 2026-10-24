//! Dev Mode in the UI, the tail: the compare view compares with a saved version picked at its
//! bottom, and falls back when that version goes away.

use figcraft_engine::Session;
use serde_json::json;

use crate::dev_compare::CompareView;
use crate::present_tests::{Ui, id};

#[test]
fn compare_view_compares_with_a_saved_version() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "Login"})).unwrap());
    let r = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 300, "width": 120, "height": 40, "parent": f})).unwrap());
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.frame(vec![]);
    // Neither ready for dev nor any version: nothing to compare with, the view doesn't stay.
    u.app.ui.dev.compare = Some(CompareView::new(f, None));
    u.frames(2, Vec::new);
    assert!(u.app.ui.dev.compare.is_none());
    let v = u.run("file.saveVersion", json!({"name": "Kickoff"}));
    let v = v["id"].as_str().unwrap().to_string();
    u.run("node.set", json!({"id": r, "props": {"width": 160}}));
    u.run("dev.compare", json!({"id": f, "version": v, "show": true}));
    u.frames(2, Vec::new);
    assert_eq!(u.app.inspect()["devCompare"]["version"], json!(v));
    // Opened without one, a layer that isn't ready compares with the newest version.
    u.app.ui.dev.compare = Some(CompareView::new(f, None));
    u.frames(2, Vec::new);
    assert_eq!(u.app.inspect()["devCompare"]["version"], json!(v));
    // Ready for dev: the snapshot from when it was marked is there too, and a deleted version
    // falls back to it.
    u.run("node.set", json!({"id": f, "props": {"devStatus": "readyForDev"}}));
    u.run("node.set", json!({"id": r, "props": {"height": 48}}));
    u.run("version.delete", json!({"id": v}));
    u.frames(2, Vec::new);
    assert_eq!(u.app.inspect()["devCompare"]["version"], json!(null));
    assert!(u.app.ui.dev.compare.is_some());
}

#[test]
fn dev_mode_keys_refuse_design_edits_and_say_why() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "Login"})).unwrap());
    let r = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 30, "width": 120, "height": 40, "parent": f})).unwrap());
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.frame(vec![]);
    let at = egui::pos2(700.0, 450.0);
    let x = |u: &Ui| u.app.session.doc().unwrap().node(r).map(|n| n.transform.as_coeffs()[4]);
    for key in [egui::Key::Delete, egui::Key::ArrowRight, egui::Key::Num5, egui::Key::R] {
        u.key(at, key);
    }
    assert!(u.app.session.doc().unwrap().contains(r), "not deleted");
    assert_eq!(x(&u), Some(20.0), "not nudged");
    assert_eq!(u.app.session.doc().unwrap().node(r).map(|n| n.opacity), Some(1.0), "opacity kept");
    assert_eq!(u.app.session.tool.tool, figcraft_engine::tools::ToolId::Move, "no drawing tool");
    assert!(u.app.ui.toast.as_ref().is_some_and(|(m, _)| m.contains("Shift+D")));
    // In the design the same keys edit.
    u.app.ui.right_tab = crate::RightTab::Design;
    u.frame(vec![]);
    u.key(at, egui::Key::ArrowRight);
    assert_eq!(x(&u), Some(21.0));
}

#[test]
fn read_only_plugins_run_from_the_inspect_tab_show_their_panel() {
    use base64::Engine as _;
    let wasm = base64::engine::general_purpose::STANDARD.encode(crate::plugin_panel_tests::panel_module());
    let manifest = json!({"id": "org.test.inspect", "name": "Inspect panel", "main": "p.wasm", "api": 2, "permissions": {"access": "read"}});
    let mut s = Session::new();
    s.execute("plugin.import", &json!({"manifest": manifest, "wasm": wasm})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.frame(vec![]);
    // What the Inspect tab's Plugins row runs.
    crate::plugins_ui::start(&mut u.app, "org.test.inspect", "run");
    u.frame(vec![]);
    assert_eq!(u.app.session.plugins.panels.len(), 1, "its panel shows");
    // Its callbacks only read: the button that would draw is refused and the design stays.
    let layers = |u: &Ui| u.app.session.doc().unwrap().ids().count();
    let before = layers(&u);
    assert!(u.app.session.execute("plugin.event", &json!({"id": "org.test.inspect", "control": "go"})).is_err());
    assert_eq!(layers(&u), before);
}

/// A clipboard holding only text.
struct TextClip(String);

impl crate::Services for TextClip {
    fn clipboard_get_text(&mut self) -> Option<String> {
        Some(self.0.clone())
    }
}

#[test]
fn dev_mode_pastes_only_a_comments_link() {
    let mut s = Session::new();
    let c = s.execute("comment.add", &json!({"x": 10, "y": 10, "text": "Check the spacing"})).unwrap()["id"].as_u64().unwrap();
    let layers = |app: &crate::FigApp| app.session.doc().unwrap().node(app.session.page().unwrap()).unwrap().children.len();
    let mut app = crate::FigApp::new(s, Box::new(TextClip("Pasted words".into())));
    app.ui.right_tab = crate::RightTab::Inspect;
    crate::menus::paste(&mut app, None, None);
    assert_eq!(layers(&app), 0, "text doesn't paste into the design in Dev Mode");
    app.services = Box::new(TextClip(format!("figcraft://comment/{c}")));
    crate::menus::paste(&mut app, None, None);
    assert_eq!(app.ui.comments.open, Some(c), "the link opens its thread");
}

#[test]
fn dev_mode_layers_panel_selects_and_expands_but_edits_nothing() {
    use egui::{Event, Pos2};
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "Login"})).unwrap());
    let a = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 30, "width": 120, "height": 40, "parent": f})).unwrap());
    let b = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 90, "width": 120, "height": 40, "parent": f})).unwrap());
    s.execute("select.none", &json!({})).unwrap();
    let mut u = crate::layers_drag_tests::Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.app.ui.expanded.insert(f);
    u.frame(vec![]);
    let order = |u: &crate::layers_drag_tests::Ui| u.app.session.doc().unwrap().node(f).unwrap().children.clone();
    let before = order(&u);
    let (ra, rb) = (u.row_at(a), u.row_at(b));
    let click = |u: &mut crate::layers_drag_tests::Ui, p: Pos2| {
        u.button(p, true);
        u.button(p, false);
    };
    let rest = |u: &mut crate::layers_drag_tests::Ui| {
        for _ in 0..30 {
            u.frame(vec![]);
        }
    };
    // A click selects.
    click(&mut u, ra);
    assert_eq!(u.app.session.selection(), [a]);
    // A double click doesn't rename, and says why.
    click(&mut u, ra);
    assert!(u.app.ui.rename.is_none(), "no rename");
    assert!(u.app.ui.toast.as_ref().is_some_and(|(m, _)| m.contains("Shift+D")));
    rest(&mut u);
    // The eye and padlock spots don't hide or lock (the row is selected instead).
    let (eye, padlock) = (272.0, 250.0);
    for x in [eye, padlock] {
        u.frame(vec![Event::PointerMoved(Pos2::new(x, rb.y))]);
        click(&mut u, Pos2::new(x, rb.y));
        rest(&mut u);
    }
    let doc = u.app.session.doc().unwrap();
    assert!(doc.node(b).unwrap().visible && !doc.node(b).unwrap().locked, "not hidden or locked");
    assert_eq!(u.app.session.selection(), [b]);
    // Dragging a row doesn't reorder.
    u.drag_to(ra, rb + egui::vec2(0.0, 6.0));
    u.button(rb + egui::vec2(0.0, 6.0), false);
    rest(&mut u);
    assert_eq!(order(&u), before, "not reordered");
    // Back in the design, the same double click renames.
    u.app.ui.right_tab = crate::RightTab::Design;
    u.frame(vec![]);
    click(&mut u, ra);
    click(&mut u, ra);
    assert!(u.app.ui.rename.is_some_and(|r| r.id == a));
}

#[test]
fn measurements_kept_on_the_canvas_draw_in_dev_mode_and_follow_their_layers() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 400})).unwrap());
    let a = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 30, "width": 120, "height": 40, "parent": f})).unwrap());
    let b = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 90, "width": 120, "height": 40, "parent": f})).unwrap());
    let mut u = Ui::new(s);
    u.run("devMode.addMeasurement", json!({"from": a, "to": b}));
    let labels = |u: &Ui| crate::dev_mode::saved_lines(&u.app).filter_map(|g| g.label.clone()).collect::<Vec<_>>();
    u.frame(vec![]);
    assert!(labels(&u).is_empty(), "the design doesn't show them");
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.frame(vec![]);
    assert_eq!(labels(&u), ["20"]);
    u.run("node.set", json!({"id": b, "props": {"y": 110}}));
    u.frame(vec![]);
    assert_eq!(labels(&u), ["40"], "it follows the layer");
    // Hidden layers' measurements aren't drawn.
    u.run("node.set", json!({"id": b, "props": {"visible": false}}));
    u.frame(vec![]);
    assert!(labels(&u).is_empty());
}
