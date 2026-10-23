//! Dev Mode in the UI, driven headless (the harness is `present_tests`'): the compare view opens
//! from `dev.compare {show}`, switches to the overlay and closes with Escape before the focus view
//! does; the focus view's Escape; the annotation filter.

use egui::{Key, pos2};
use figcraft_engine::Session;
use serde_json::json;

use crate::dev_compare::Mode;
use crate::dev_mode::CategoryFilter;
use crate::present_tests::{Ui, id};

fn setup() -> (Ui, figcraft_doc::NodeId, figcraft_doc::NodeId) {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "Login"})).unwrap());
    let r = id(&s
        .execute("shape.rectangle", &json!({"x": 20, "y": 300, "width": 120, "height": 40, "parent": f, "props": {"fill": "#FF0000"}}))
        .unwrap());
    s.execute("node.set", &json!({"id": f, "props": {"devStatus": "readyForDev"}})).unwrap();
    s.execute("node.set", &json!({"id": r, "props": {"fill": "#00AA00", "width": 160}})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.frame(vec![]);
    (u, f, r)
}

#[test]
fn compare_view_opens_switches_and_closes_before_the_focus_view() {
    let (mut u, f, _) = setup();
    u.run("dev.focus", json!({"id": f}));
    u.run("dev.compare", json!({"id": f, "show": true}));
    assert_eq!(u.app.inspect()["devCompare"]["id"], json!(f.0));
    assert_eq!(u.app.inspect()["devFocus"], json!(f.0));
    if let Some(c) = u.app.ui.dev.compare.as_mut() {
        c.mode = Mode::Overlay;
        c.split = 0.3;
    }
    u.frames(2, Vec::new);
    assert_eq!(u.app.inspect()["devCompare"]["mode"], json!("overlay"));
    // Escape closes the compare view only; the next one leaves the focus view.
    let at = pos2(700.0, 450.0);
    u.key(at, Key::Escape);
    u.frame(vec![]);
    assert!(u.app.ui.dev.compare.is_none());
    assert_eq!(u.app.inspect()["devFocus"], json!(f.0));
    u.key(at, Key::Escape);
    u.frame(vec![]);
    assert_eq!(u.app.inspect()["devFocus"], json!(null));
}

#[test]
fn compare_view_follows_edits_and_resolving() {
    let (mut u, f, r) = setup();
    u.run("dev.compare", json!({"id": f, "show": true}));
    u.run("node.set", json!({"id": r, "props": {"height": 48}}));
    u.frames(2, Vec::new);
    assert!(u.app.ui.dev.compare.is_some(), "still open after an edit");
    u.run("dev.resolveChange", json!({"ids": [f]}));
    u.frames(2, Vec::new);
    // Nothing changed since: the view says so instead of comparing.
    assert!(u.app.ui.dev.compare.is_some());
    u.run("edit.delete", json!({"ids": [f]}));
    u.frames(2, Vec::new);
    assert!(u.app.ui.dev.compare.is_none(), "closes when its frame is gone");
}

#[test]
fn annotation_filter_and_categories_panel() {
    let (mut u, _, r) = setup();
    u.run("annotation.add", json!({"id": r, "label": "Tap target", "category": "Accessibility"}));
    u.run("annotation.add", json!({"id": r, "label": "Plain"}));
    u.run("select.set", json!({"ids": []}));
    for f in [CategoryFilter::All, CategoryFilter::Uncategorized, CategoryFilter::Only("accessibility".into())] {
        u.app.ui.dev.categories = f.clone();
        u.frames(2, Vec::new);
        assert_eq!(u.app.inspect()["annotationFilter"], json!(f.id()));
    }
    let doc = u.app.session.doc().unwrap().clone();
    assert_eq!(CategoryFilter::parse(&doc, "Accessibility"), Some(CategoryFilter::Only("accessibility".into())));
    assert_eq!(CategoryFilter::parse(&doc, "none"), Some(CategoryFilter::Uncategorized));
    assert_eq!(CategoryFilter::parse(&doc, "nope"), None);
}

#[test]
fn the_assets_list_shows_a_slice_with_a_preview_name_and_export_format() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 200, "name": "Login"})).unwrap());
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 80, "height": 40, "parent": f, "props": {"fill": "#FF0000"}})).unwrap();
    // A slice with one export setting: its own format is what the row offers first.
    let slice = id(&s.execute("node.create", &json!({"type": "slice", "x": 10, "y": 10, "width": 100, "height": 60, "name": "Avatar"})).unwrap());
    s.execute("export.add", &json!({"ids": [slice], "setting": {"format": "webp", "size": "2x"}})).unwrap();
    s.execute("select.set", &json!({"ids": [slice]})).unwrap();
    let mut u = crate::layers_drag_tests::Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.ctx.enable_accesskit();
    let out = u.frame_output(vec![]);
    let nodes: Vec<egui::accesskit::Node> =
        out.platform_output.accesskit_update.expect("an AccessKit update").nodes.into_iter().map(|(_, n)| n).collect();
    let find = |role: egui::accesskit::Role, label: &str| {
        nodes.iter().find(|n| n.role() == role && n.label() == Some(label)).unwrap_or_else(|| panic!("no {role:?} named {label:?}"))
    };
    // The row exports by a click, and the format under the name is a dropdown.
    find(egui::accesskit::Role::Button, "Export Avatar");
    let fmt = find(egui::accesskit::Role::ComboBox, "Format");
    assert_eq!(fmt.value(), Some("WebP"));
    assert_eq!(fmt.is_expanded(), Some(false));
}
