//! When the Design panel shows Selection colors (#2188): measured in Figma's web app, a layer with
//! children shows the section from one distinct colour, while layers without children need two.
//! Driven through egui headless; each row has a "Select layers with this color" button.

use egui::accesskit::Role;
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::layers_drag_tests::Ui;

const ROW_BUTTON: &str = "Select layers with this color";

fn id(v: Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: i32, color: &str, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": 0, "width": 40, "height": 40});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    let r = id(s.execute("shape.rectangle", &p).unwrap());
    s.execute("fill.set", &json!({"ids": [r], "color": color})).unwrap();
    r
}

fn frame(s: &mut Session, fill: Option<&str>) -> NodeId {
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap());
    match fill {
        Some(c) => s.execute("fill.set", &json!({"ids": [f], "color": c})).unwrap(),
        None => s.execute("fill.remove", &json!({"ids": [f], "index": 0})).unwrap(),
    };
    f
}

/// How many Selection colors rows the Design panel shows with `ids` selected.
fn rows(mut s: Session, ids: &[NodeId]) -> usize {
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    nodes.iter().filter(|(_, n)| n.role() == Role::Button && n.label() == Some(ROW_BUTTON)).count()
}

#[test]
fn a_frame_holding_one_red_layer_shows_selection_colors_with_one_row() {
    let mut s = Session::new();
    let f = frame(&mut s, None);
    rect(&mut s, 10, "#FF0000", Some(f));
    assert_eq!(rows(s, &[f]), 1);
}

#[test]
fn a_group_of_two_red_layers_shows_one_row() {
    let mut s = Session::new();
    let a = rect(&mut s, 0, "#FF0000", None);
    let b = rect(&mut s, 60, "#FF0000", None);
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let g = id(s.execute("object.group", &json!({"ids": [a, b]})).unwrap());
    assert_eq!(rows(s, &[g]), 1);
}

#[test]
fn an_instance_whose_child_has_one_fill_shows_selection_colors() {
    let mut s = Session::new();
    let f = frame(&mut s, None);
    rect(&mut s, 10, "#FF0000", Some(f));
    let c = id(s.execute("component.create", &json!({"ids": [f]})).unwrap());
    let i = id(s.execute("instance.create", &json!({"component": c, "x": 400, "y": 0})).unwrap());
    assert_eq!(rows(s, &[i]), 1);
}

#[test]
fn one_layer_or_two_of_the_same_colour_without_children_hide_it() {
    let mut s = Session::new();
    let a = rect(&mut s, 0, "#FF0000", None);
    assert_eq!(rows(s, &[a]), 0, "one red rectangle");
    let mut s = Session::new();
    let a = rect(&mut s, 0, "#FF0000", None);
    let b = rect(&mut s, 60, "#FF0000", None);
    assert_eq!(rows(s, &[a, b]), 0, "two red rectangles");
    let mut s = Session::new();
    let g = frame(&mut s, Some("#00FF00"));
    assert_eq!(rows(s, &[g]), 0, "a childless green frame");
}

#[test]
fn two_loose_layers_of_different_colours_show_two_rows() {
    let mut s = Session::new();
    let a = rect(&mut s, 0, "#FF0000", None);
    let b = rect(&mut s, 60, "#0000FF", None);
    assert_eq!(rows(s, &[a, b]), 2);
}
