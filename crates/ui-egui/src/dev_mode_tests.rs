//! Dev Mode in the UI, driven headless (the harness is `present_tests`'): the compare view opens
//! from `dev.compare {show}`, switches to the overlay and closes with Escape before the focus view
//! does; the focus view's Escape; the annotation filter; the playground's instance-swap menu.

use egui::accesskit::{Node, Role};
use egui::{Event, Key, MouseWheelUnit, Pos2, TouchPhase, pos2, vec2};
use figcraft_doc::NodeId;
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
    // A format picked there stays after the document changes.
    let at = centre(fmt);
    click(&mut u, at);
    let svg = tree(&mut u).into_iter().find(|n| n.label() == Some("SVG")).expect("SVG in the format menu");
    click(&mut u, centre(&svg));
    let revision = u.app.session.ds().unwrap().revision;
    u.app.session.execute("node.set", &json!({"ids": [slice], "props": {"name": "Avatar"}})).unwrap();
    u.app.session.execute("node.set", &json!({"ids": [f], "props": {"name": "Sign in"}})).unwrap();
    assert_ne!(u.app.session.ds().unwrap().revision, revision);
    let fmt = tree(&mut u).into_iter().find(|n| n.role() == Role::ComboBox && n.label() == Some("Format")).unwrap();
    assert_eq!(fmt.value(), Some("SVG"));
}

/// A "Button" component whose nested "Heart" instance is its instance-swap property "Icon", a
/// second icon "Star", and an instance of the button selected in the Inspect tab. Returns (ui,
/// instance, heart, star).
fn swap_setup() -> (crate::layers_drag_tests::Ui, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    let icon = |s: &mut Session, name: &str, x: i32| {
        let f = id(&s.execute("frame.create", &json!({"x": x, "y": 0, "width": 24, "height": 24, "name": name})).unwrap());
        id(&s.execute("component.create", &json!({"ids": [f]})).unwrap())
    };
    let (heart, star) = (icon(&mut s, "Heart", 0), icon(&mut s, "Star", 40));
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 100, "width": 120, "height": 40, "name": "Button"})).unwrap());
    let nested = id(&s.execute("instance.create", &json!({"component": heart, "x": 8, "y": 108, "parent": f})).unwrap());
    let button = id(&s.execute("component.create", &json!({"ids": [f]})).unwrap());
    s.execute("component.addProperty", &json!({"id": button, "name": "Icon", "type": "instanceSwap", "default": heart, "layer": nested})).unwrap();
    let inst = id(&s.execute("instance.create", &json!({"component": button, "x": 0, "y": 300})).unwrap());
    s.execute("select.set", &json!({"ids": [inst]})).unwrap();
    let mut u = crate::layers_drag_tests::Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.ctx.enable_accesskit();
    (u, inst, heart, star)
}

fn tree(u: &mut crate::layers_drag_tests::Ui) -> Vec<Node> {
    u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes.into_iter().map(|(_, n)| n).collect()
}

fn centre(n: &Node) -> Pos2 {
    let b = n.bounds().unwrap();
    pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

fn click(u: &mut crate::layers_drag_tests::Ui, at: Pos2) {
    u.button(at, true);
    u.button(at, false);
}

/// The playground's dropdown for property `name`, scrolled into view.
fn playground_combo(u: &mut crate::layers_drag_tests::Ui, name: &str) -> Option<Node> {
    for _ in 0..20 {
        let combo = tree(u).into_iter().find(|n| n.role() == Role::ComboBox && n.label() == Some(name))?;
        if centre(&combo).y < 860.0 {
            return Some(combo);
        }
        let wheel = Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            source: egui::MouseWheelSource::Unknown,
            delta: vec2(0.0, -100.0),
            phase: TouchPhase::Move,
            modifiers: Default::default(),
        };
        u.frame(vec![Event::PointerMoved(pos2(1300.0, 500.0)), wheel]);
        for _ in 0..30 {
            u.frame(vec![]);
        }
    }
    None
}

#[test]
fn the_playground_swaps_an_instance_in_the_preview_only() {
    let (mut u, inst, heart, star) = swap_setup();
    let (doc, undo) = (u.app.session.ds().unwrap().doc.clone(), u.app.session.undo_depth());
    let combo = playground_combo(&mut u, "Icon").expect("the playground has a dropdown for the instance-swap property");
    assert_eq!(combo.value(), Some("Heart"));
    // Open it and pick Star: the item that wasn't there before the menu opened.
    let before: Vec<Pos2> = tree(&mut u).iter().filter(|n| n.label() == Some("Star")).map(centre).collect();
    click(&mut u, centre(&combo));
    let item = tree(&mut u).into_iter().find(|n| n.label() == Some("Star") && !before.contains(&centre(n))).expect("Star in the menu");
    click(&mut u, centre(&item));
    let combo = playground_combo(&mut u, "Icon").unwrap();
    assert_eq!(combo.value(), Some("Star"), "the playground shows the swapped icon");
    // The file isn't touched: the instance still has the heart, nothing to undo.
    assert!(std::sync::Arc::ptr_eq(&doc, &u.app.session.ds().unwrap().doc));
    assert_eq!(u.app.session.undo_depth(), undo);
    let props = u.app.session.execute("component.properties", &json!({"id": inst})).unwrap();
    assert_eq!(props["properties"][0]["value"], json!(heart.0), "{star:?}");
}
