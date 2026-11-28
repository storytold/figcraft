//! Guides dragged out of a ruler snap to nearby layer edges and centres (Snap to Objects), on the
//! guide's own axis, and Cmd/Ctrl suspends that while the guides keep their pixel snapping.

use egui::{Event, Modifiers, Pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rulers(mut s: Session) -> Ui {
    s.execute("view.rulers", &json!({"on": true})).unwrap();
    Ui::new(s)
}

fn guides(u: &Ui, owner: NodeId) -> Vec<(bool, f64)> {
    let ds = u.app.session.ds().unwrap();
    ds.doc.node(owner).unwrap().guides().iter().map(|g| (g.vertical, g.offset)).collect()
}

fn page(u: &Ui) -> NodeId {
    u.app.session.ds().unwrap().page
}

fn from_top_ruler(u: &Ui) -> Pos2 {
    let r = u.app.canvas.rect;
    Pos2::new(r.min.x + r.width() * 0.5, r.min.y + 5.0)
}

fn from_left_ruler(u: &Ui) -> Pos2 {
    let r = u.app.canvas.rect;
    Pos2::new(r.min.x + 5.0, r.min.y + r.height() * 0.5)
}

fn drag(u: &mut Ui, from: Pos2, to: Pos2) {
    u.frame(vec![Event::PointerMoved(from)]);
    u.button(from, true);
    u.frame(vec![Event::PointerMoved(to)]);
    u.button(to, false);
}

fn drag_with(u: &mut Ui, from: Pos2, to: Pos2, m: Modifiers) {
    u.frame_with(vec![Event::PointerMoved(from)], m);
    u.frame_with(vec![Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: m }], m);
    u.frame_with(vec![Event::PointerMoved(to)], m);
    u.frame_with(vec![Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: m }], m);
}

#[test]
fn a_guide_dragged_from_a_ruler_snaps_to_a_layers_edge() {
    let mut s = Session::new();
    s.execute("node.create", &json!({"type": "rectangle", "x": 100, "y": 100, "width": 120, "height": 80})).unwrap();
    let mut u = rulers(s);
    let start = from_top_ruler(&u);
    let to = u.app.to_screen(200.0, 102.0);
    drag(&mut u, start, to);
    assert_eq!(guides(&u, page(&u)), vec![(false, 100.0)], "two pixels from the top edge");
}

#[test]
fn a_guide_dragged_over_a_frame_snaps_to_a_layer_inside_it() {
    let mut s = Session::new();
    let frame = id(s.execute("node.create", &json!({"type": "frame", "x": 100, "y": 100, "width": 300, "height": 200})).unwrap());
    s.execute("node.create", &json!({"type": "rectangle", "parent": frame, "x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    let mut u = rulers(s);
    let start = from_left_ruler(&u);
    let to = u.app.to_screen(112.0, 200.0);
    drag(&mut u, start, to);
    assert_eq!(guides(&u, frame), vec![(true, 10.0)], "two pixels from the child's left edge");
}

#[test]
fn with_snap_to_objects_off_the_guide_keeps_its_pixel_rounding() {
    let mut s = Session::new();
    s.execute("node.create", &json!({"type": "rectangle", "x": 100, "y": 100, "width": 120, "height": 80})).unwrap();
    s.execute("view.snapObjects", &json!({"on": false})).unwrap();
    let mut u = rulers(s);
    let start = from_top_ruler(&u);
    let to = u.app.to_screen(200.0, 102.0);
    drag(&mut u, start, to);
    assert_eq!(guides(&u, page(&u)), vec![(false, 102.0)], "rounded to the pixel, not the edge");
}

#[test]
fn a_click_on_a_guide_only_selects_it() {
    let mut s = Session::new();
    s.execute("node.create", &json!({"type": "rectangle", "x": 100, "y": 100, "width": 120, "height": 80})).unwrap();
    s.execute("guide.add", &json!({"vertical": false, "offset": 102})).unwrap();
    let mut u = rulers(s);
    // Two pixels above the rectangle's top edge, but a click without movement leaves it there.
    let at = u.app.to_screen(500.0, 102.0);
    u.button(at, true);
    u.button(at, false);
    assert_eq!(guides(&u, page(&u)), vec![(false, 102.0)], "a click must not snap it to the edge at 100");
}

#[test]
fn cmd_or_ctrl_suspends_guide_snapping() {
    let mut s = Session::new();
    s.execute("node.create", &json!({"type": "rectangle", "x": 100, "y": 100, "width": 120, "height": 80})).unwrap();
    let mut u = rulers(s);
    let start = from_top_ruler(&u);
    let to = u.app.to_screen(200.0, 102.0);
    drag_with(&mut u, start, to, Modifiers::CTRL);
    assert_eq!(guides(&u, page(&u)), vec![(false, 102.0)]);
}
