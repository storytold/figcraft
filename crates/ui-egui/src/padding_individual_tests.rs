//! Individual padding in the Design panel, driven headless: the four side fields only
//! appear once the toggle opens, and typing in one sets that side alone.

use egui::accesskit::Role;
use egui::{Event, Id, Key, Modifiers, Pos2, pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

/// An auto layout frame with uniform padding 10, selected.
fn scene() -> (Ui, NodeId) {
    let mut s = Session::new();
    let f =
        NodeId(
            s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 200, "props": {"layout": "horizontal", "padding": 10}}))
                .unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    (Ui::new(s), f)
}

/// The frame's padding as `[top, right, bottom, left]`.
fn padding(u: &Ui, f: NodeId) -> [f64; 4] {
    u.app.session.doc().unwrap().node(f).unwrap().layout.clone().unwrap().padding
}

/// A full press-and-release at `p`, then settling one frame.
fn click(u: &mut Ui, p: Pos2) {
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![]);
}

/// A press-and-release of Enter, committing the focused number field.
fn enter(u: &mut Ui) {
    u.frame(vec![
        Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
        Event::Key { key: Key::Enter, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
    ]);
}

/// The middle of the toggle button named `label`, found in the AccessKit tree.
fn named_button(u: &mut Ui, label: &str) -> Pos2 {
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let n = nodes.iter().map(|(_, n)| n).find(|n| n.role() == Role::Button && n.label() == Some(label)).expect(label);
    let b = n.bounds().expect("bounds");
    pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

/// The side fields stay hidden until the toggle opens, then typing in one sets
/// that side alone in a single undo step.
#[test]
fn side_fields_show_only_once_the_toggle_opens_and_set_one_side() {
    let (mut u, f) = scene();
    assert_eq!(padding(&u, f), [10.0; 4]);
    // Shut: no per-side fields.
    assert!(u.ctx.read_response(Id::unique(("padding", 0)).with("field")).is_none(), "top field hidden until toggled");
    // Click the toggle: the four sides appear.
    let t = named_button(&mut u, "Individual padding");
    click(&mut u, t);
    let top = u.ctx.read_response(Id::unique(("padding", 0)).with("field")).expect("top field shows").rect.center();
    assert!(u.ctx.read_response(Id::unique(("padding", 3)).with("field")).is_some(), "left field shows");
    // Typing in the top field sets only the top, in one undo step.
    let history = u.app.session.undo_depth().0;
    click(&mut u, top);
    u.frame(vec![Event::Text("8".into())]);
    enter(&mut u);
    u.frame(vec![]);
    assert_eq!(padding(&u, f), [8.0, 10.0, 10.0, 10.0]);
    assert_eq!(u.app.session.undo_depth().0, history + 1);
    u.app.run("edit.undo", json!({}));
    assert_eq!(padding(&u, f), [10.0; 4]);
}

/// The middle of the shut pair field with accessible name `label`.
fn named_field(u: &mut Ui, label: &str) -> Pos2 {
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let n = nodes.iter().map(|(_, n)| n).find(|n| n.role() != Role::Button && n.label() == Some(label)).expect(label);
    let b = n.bounds().expect("bounds");
    pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

/// The shut pair fields read Mixed when their sides disagree and edit their own axis: horizontal
/// touches left and right only, vertical touches top and bottom only.
#[test]
fn shut_pair_fields_edit_their_own_axis() {
    let (mut u, f) = scene();
    u.app.run("node.set", json!({"ids": [f], "props": {"paddingLeft": 16.0, "paddingRight": 54.0}}));
    assert_eq!(padding(&u, f), [10.0, 54.0, 10.0, 16.0]);
    let shown = crate::live_tests::texts(&crate::live_tests::flat(&u.frame_output(vec![]).shapes));
    assert!(shown.iter().any(|(t, _, _)| t == "Mixed"), "sides that disagree read Mixed: {shown:?}");
    let h = named_field(&mut u, "Horizontal padding");
    click(&mut u, h);
    u.frame(vec![Event::Text("20".into())]);
    enter(&mut u);
    u.frame(vec![]);
    assert_eq!(padding(&u, f), [10.0, 20.0, 10.0, 20.0]);
    let v = named_field(&mut u, "Vertical padding");
    click(&mut u, v);
    u.frame(vec![Event::Text("7".into())]);
    enter(&mut u);
    u.frame(vec![]);
    assert_eq!(padding(&u, f), [7.0, 20.0, 7.0, 20.0]);
}
