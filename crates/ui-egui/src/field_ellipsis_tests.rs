//! A text field that isn't being edited ends a value too long for it with an ellipsis, shows the
//! whole value on hover, and keeps it whole for screen readers (#2368: the variant value "Down
//! left" showed as "Down lef" in a 62 px chip).

use egui::accesskit::Node;
use egui::epaint::{ClippedShape, Shape};
use egui::{Event, Pos2, pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::component_swap_default_tests::tree;
use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A selected component set with a "Looking" property of `values`.
fn set_with(values: &[&str]) -> Ui {
    let mut s = Session::new();
    let mut variants = Vec::new();
    for (k, v) in values.iter().enumerate() {
        let f = id(s.execute("frame.create", &json!({"x": 0, "y": k * 60, "width": 80, "height": 40, "name": format!("Looking={v}")})).unwrap());
        variants.push(id(s.execute("component.create", &json!({"ids": [f]})).unwrap()));
    }
    let set = id(s.execute("component.combineVariants", &json!({"ids": variants})).unwrap());
    s.execute("select.set", &json!({"ids": [set]})).unwrap();
    let u = Ui::new(s);
    u.ctx.enable_accesskit();
    u
}

/// How many painted texts read `text`.
fn count(shapes: &[String], text: &str) -> usize {
    shapes.iter().filter(|t| *t == text).count()
}

/// Frames with a still pointer on `at`, long enough for egui's tooltip delay; the last one's texts.
fn hover(u: &mut Ui, at: Pos2) -> Vec<String> {
    let mut shown = Vec::new();
    u.frame(vec![Event::PointerMoved(at)]);
    for _ in 0..90 {
        shown = texts(&u.frame_output(vec![]).shapes);
    }
    shown
}

fn texts(shapes: &[ClippedShape]) -> Vec<String> {
    fn walk(s: &Shape, out: &mut Vec<String>) {
        match s {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            Shape::Text(t) => out.push(t.galley.text().to_owned()),
            _ => {}
        }
    }
    let mut out = Vec::new();
    shapes.iter().for_each(|c| walk(&c.shape, &mut out));
    out
}

/// The accesskit node of the value chip showing `value`.
fn chip(nodes: &[Node], value: &str) -> Pos2 {
    let n = nodes.iter().find(|n| n.value() == Some(value) && n.bounds().is_some()).unwrap_or_else(|| panic!("a chip for {value}"));
    let b = n.bounds().unwrap();
    pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

#[test]
fn a_variant_value_too_long_for_its_chip_ends_with_an_ellipsis() {
    let mut u = set_with(&["Top right", "Down left"]);
    let shown = texts(&u.frame_output(vec![]).shapes);
    let cut: Vec<&String> = shown.iter().filter(|t| t.starts_with("Down l")).collect();
    assert!(!cut.is_empty(), "the value is painted: {shown:?}");
    assert!(cut.iter().all(|t| t.ends_with('…')), "ends with an ellipsis: {cut:?}");
    assert!(!shown.iter().any(|t| t == "Down lef"), "not cut mid-letter: {shown:?}");
}

#[test]
fn the_shortened_value_shows_whole_on_hover() {
    let mut u = set_with(&["Top right", "Down left"]);
    let at = chip(&tree(&mut u), "Down left");
    let before = count(&texts(&u.frame_output(vec![]).shapes), "Down left");
    let shown = hover(&mut u, at);
    assert_eq!(count(&shown, "Down left"), before + 1, "the whole value is painted again, in a tooltip: {shown:?}");
}

#[test]
fn a_value_that_fits_is_shown_whole_without_a_tooltip() {
    let mut u = set_with(&["Top", "Down left"]);
    let nodes = tree(&mut u);
    let shown = texts(&u.frame_output(vec![]).shapes);
    let before = count(&shown, "Top");
    assert!(before > 0, "Top is painted as it is: {shown:?}");
    let after = hover(&mut u, chip(&nodes, "Top"));
    assert_eq!(count(&after, "Top"), before, "no tooltip repeats it: {after:?}");
}

#[test]
fn screen_readers_keep_the_whole_value() {
    let mut u = set_with(&["Top right", "Down left"]);
    let nodes = tree(&mut u);
    assert!(nodes.iter().any(|n| n.value() == Some("Down left")), "the chip's value is the whole text");
    assert!(!nodes.iter().any(|n| n.value().is_some_and(|v| v.ends_with('…'))), "no node announces a shortened value");
}
