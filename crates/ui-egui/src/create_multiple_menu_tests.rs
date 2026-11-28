//! With two or more layers selected the Design panel's component button opens a menu (Create
//! component, Create multiple components, Create component set); one layer keeps the plain click (#2088): drawn headless.

use egui::accesskit::Node;
use egui::{Event, PointerButton, Pos2, Rect, pos2};
use figcraft_doc::{NodeId, NodeKind};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

const NAMES: [&str; 3] = ["Card", "Badge", "Tile"];

fn id(v: serde_json::Value) -> NodeId {
    serde_json::from_value(v["id"].clone()).unwrap()
}

/// Frames named `NAMES[..n]`, all selected, headless.
fn frames(n: usize) -> (Ui, Vec<NodeId>) {
    let mut s = Session::new();
    let mut ids = Vec::new();
    for (i, name) in NAMES.into_iter().take(n).enumerate() {
        let f = id(s.execute("frame.create", &json!({"x": 150 * i, "y": 0, "width": 100, "height": 60})).unwrap());
        s.execute("object.rename", &json!({"ids": [f], "name": name})).unwrap();
        ids.push(f);
    }
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let mut u = Ui::new(s);
    u.frame(vec![]);
    (u, ids)
}

fn nodes(u: &mut Ui) -> Vec<Node> {
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    u.frame_output(vec![]).platform_output.accesskit_update.expect("an AccessKit update").nodes.into_iter().map(|(_, n)| n).collect()
}

fn bounds(n: &Node) -> Rect {
    let b = n.bounds().unwrap();
    Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32))
}

fn click(u: &mut Ui, at: Pos2) {
    for pressed in [true, false] {
        u.frame(vec![
            Event::PointerMoved(at),
            Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() },
        ]);
    }
}

fn settle(u: &mut Ui) {
    for _ in 0..10 {
        u.frame(vec![]);
    }
}

/// The header's component button (its tooltip may carry the shortcut).
fn click_button(u: &mut Ui) {
    let all = nodes(u);
    let button = all.iter().find(|n| n.label().is_some_and(|l| l.starts_with("Create component"))).expect("the Design panel's component button");
    click(u, bounds(button).center());
    settle(u);
}

fn undo_len(u: &Ui) -> usize {
    u.app.session.ds().unwrap().history.undo.len()
}

#[test]
fn several_layers_open_a_menu_and_create_multiple_components_in_one_undo_step() {
    let (mut u, ids) = frames(3);
    let before = undo_len(&u);
    click_button(&mut u);
    assert_eq!(undo_len(&u), before, "opening the menu changes nothing");
    let all = nodes(&mut u);
    // The menu's rows share an x with Create multiple components: exactly these two, in this order.
    let multiple = all.iter().find(|n| n.label() == Some("Create multiple components")).expect("the menu's second entry");
    let mb = bounds(multiple);
    let mut rows: Vec<(f32, String)> = all
        .iter()
        .filter(|n| n.label().is_some() && (bounds(n).min.x - mb.min.x).abs() < 1.0 && (bounds(n).min.y - mb.min.y).abs() < 90.0)
        .map(|n| (bounds(n).min.y, n.label().unwrap_or("").to_owned()))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let labels: Vec<&str> = rows.iter().map(|(_, l)| l.as_str()).collect();
    assert_eq!(labels, ["Create component", "Create multiple components", "Create component set"], "the menu's entries");
    click(&mut u, bounds(multiple).center());
    settle(&mut u);
    let sel = u.app.session.selection();
    assert_eq!(sel.len(), 3);
    let doc = u.app.session.doc().unwrap();
    let names: Vec<&str> = sel.iter().map(|i| doc.node(*i).unwrap().name.as_str()).collect();
    assert_eq!(names, NAMES);
    assert!(sel.iter().all(|i| matches!(doc.node(*i).unwrap().kind, NodeKind::Component { .. })));
    assert_eq!(undo_len(&u), before + 1);
    u.app.session.undo().unwrap();
    let doc = u.app.session.doc().unwrap();
    assert!(ids.iter().all(|i| matches!(doc.node(*i).map(|n| &n.kind), Some(NodeKind::Frame))), "one undo reverts all three");
}

#[test]
fn one_layer_keeps_the_plain_button() {
    let (mut u, ids) = frames(1);
    click_button(&mut u);
    let all = nodes(&mut u);
    assert!(all.iter().all(|n| n.label() != Some("Create multiple components")), "no menu opens");
    let sel = u.app.session.selection();
    assert_eq!(sel.len(), 1);
    let doc = u.app.session.doc().unwrap();
    assert!(matches!(doc.node(sel[0]).unwrap().kind, NodeKind::Component { .. }));
    assert_eq!(doc.node(sel[0]).unwrap().name, NAMES[0]);
    let _ = ids;
}

#[test]
fn the_third_entry_creates_one_component_set() {
    let (mut u, ids) = frames(3);
    let before = undo_len(&u);
    click_button(&mut u);
    let all = nodes(&mut u);
    let entry = all.iter().find(|n| n.label() == Some("Create component set")).expect("the menu's third entry");
    click(&mut u, bounds(entry).center());
    settle(&mut u);
    let sel = u.app.session.selection();
    assert_eq!(sel.len(), 1, "the set is selected");
    let doc = u.app.session.doc().unwrap();
    let set = doc.node(sel[0]).unwrap();
    assert!(matches!(set.kind, NodeKind::ComponentSet { .. }));
    assert_eq!(set.children.len(), 3);
    assert_eq!(undo_len(&u), before + 1);
    u.app.session.undo().unwrap();
    let doc = u.app.session.doc().unwrap();
    assert!(ids.iter().all(|i| matches!(doc.node(*i).map(|n| &n.kind), Some(NodeKind::Frame))), "one undo reverts it all");
}

/// The component button's width in the Design panel: wider with a dropdown chevron beside the icon.
fn button_width(u: &mut Ui) -> f32 {
    let all = nodes(u);
    let button = all.iter().find(|n| n.label().is_some_and(|l| l.starts_with("Create component"))).expect("the Design panel's component button");
    bounds(button).width()
}

#[test]
fn the_dropdown_chevron_is_there_only_for_several_layers() {
    let (mut one, _) = frames(1);
    let (mut two, _) = frames(2);
    let (single, several) = (button_width(&mut one), button_width(&mut two));
    assert_eq!(single, 24.0, "one layer: the plain icon button");
    assert_eq!(several, 34.0, "several layers: room for the chevron");
}

#[test]
fn create_component_set_is_offered_only_where_it_works() {
    let (mut u, ids) = frames(3);
    // Make the first a component: the set can't take it, so the entry is left out.
    u.app.session.execute("component.create", &json!({"ids": [ids[0]]})).unwrap();
    u.app.session.execute("select.set", &json!({"ids": ids})).unwrap();
    u.frame(vec![]);
    click_button(&mut u);
    let all = nodes(&mut u);
    assert!(all.iter().any(|n| n.label() == Some("Create multiple components")), "the menu is open");
    assert!(all.iter().all(|n| n.label() != Some("Create component set")), "left out for a selection with a component");
}
