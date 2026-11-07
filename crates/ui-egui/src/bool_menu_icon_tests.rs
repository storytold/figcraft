//! The Design panel's boolean operation menu leads each entry with its operation's icon (#2003),
//! and each entry still runs its command: drawn headless.

use egui::accesskit::Node;
use egui::{Event, PointerButton, Pos2, Rect, Shape, pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::icons::Icon;
use crate::layers_drag_tests::Ui;

const OPS: [(&str, Icon, &str); 4] = [
    ("Union", Icon::Union, "bool.union"),
    ("Subtract", Icon::Subtract, "bool.subtract"),
    ("Intersect", Icon::Intersect, "bool.intersect"),
    ("Exclude", Icon::Exclude, "bool.exclude"),
];

fn id(v: serde_json::Value) -> NodeId {
    serde_json::from_value(v["id"].clone()).unwrap()
}

/// Two overlapping layers selected, headless.
fn two_layers() -> Ui {
    let mut s = Session::new();
    let a = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let b = id(s.execute("shape.ellipse", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap());
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let mut u = Ui::new(s);
    u.frame(vec![]);
    u
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

/// The menu open: the nodes on screen.
fn open_menu(u: &mut Ui) -> Vec<Node> {
    let all = nodes(u);
    let button = all.iter().find(|n| n.label() == Some("Boolean groups")).expect("the Design panel's Boolean groups button");
    click(u, bounds(button).center());
    for _ in 0..10 {
        u.frame(vec![]);
    }
    nodes(u)
}

/// How many strokes and fills sit in the left 40 points of `row` (where the icon is).
fn marks_in_lead(shapes: &[egui::epaint::ClippedShape], row: Rect) -> usize {
    fn walk(s: &Shape, area: Rect, n: &mut usize) {
        match s {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, area, n)),
            Shape::Path(_) | Shape::Mesh(_) | Shape::Rect(_) | Shape::LineSegment { .. } if area.contains_rect(s.visual_bounding_rect()) => *n += 1,
            _ => {}
        }
    }
    let area = Rect::from_min_max(row.min, pos2(row.min.x + 40.0, row.max.y));
    let mut n = 0;
    shapes.iter().for_each(|c| walk(&c.shape, area, &mut n));
    n
}

#[test]
fn the_boolean_menu_leads_every_operation_with_its_icon() {
    let mut u = two_layers();
    let all = open_menu(&mut u);
    let out = u.frame_output(vec![]);
    for (name, _, _) in OPS {
        let row = all.iter().find(|n| n.label() == Some(name)).unwrap_or_else(|| panic!("{name} in the menu"));
        let n = marks_in_lead(&out.shapes, bounds(row));
        assert!(n >= 2, "{name}'s icon painted at the start of its row ({n} marks)");
    }
}

#[test]
fn every_boolean_menu_entry_still_runs_its_command() {
    for (name, icon, _) in OPS {
        let mut u = two_layers();
        let all = open_menu(&mut u);
        let row = all.iter().find(|n| n.label() == Some(name)).unwrap_or_else(|| panic!("{name} in the menu"));
        click(&mut u, bounds(row).center());
        for _ in 0..3 {
            u.frame(vec![]);
        }
        let sel = u.app.session.selection();
        assert_eq!(sel.len(), 1, "{name} made one boolean group");
        assert_eq!(crate::layers::node_icon(u.app.session.doc().unwrap(), sel[0]), icon, "{name}'s group shows {icon:?}");
    }
}

/// The x every label (checked or not, with an icon or without, Flatten) starts at.
fn label_xs(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, f32)> {
    fn walk(s: &Shape, out: &mut Vec<(String, f32)>) {
        match s {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            Shape::Text(t) => out.push((t.galley.text().to_owned(), t.pos.x)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    shapes.iter().for_each(|c| walk(&c.shape, &mut out));
    out
}

#[test]
fn every_boolean_menu_label_starts_at_the_same_x() {
    for checked in [false, true] {
        let mut u = two_layers();
        if checked {
            u.app.run("bool.union", json!({}));
            u.frame(vec![]);
        }
        open_menu(&mut u);
        let out = u.frame_output(vec![]);
        let xs = label_xs(&out.shapes);
        let at: Vec<f32> = ["Union", "Subtract", "Intersect", "Exclude", "Flatten"]
            .iter()
            .map(|n| xs.iter().rev().find(|(t, _)| t == n).unwrap_or_else(|| panic!("{n} label (checked: {checked})")).1)
            .collect();
        assert!(at.iter().all(|x| (x - at[0]).abs() < 0.01), "labels line up (checked: {checked}): {at:?}");
    }
}
