//! Individual corners in the Design panel, driven headless: the four corner fields sit where
//! their corners are (top left, top right / bottom left, bottom right) and each sets its own.

use egui::accesskit::Role;
use egui::{Event, Id, Key, Modifiers, Pos2, Rect, pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

/// A rectangle rounded only on its left corners (top left and bottom left 30), selected.
fn scene() -> (Ui, NodeId) {
    let mut s = Session::new();
    let r = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 200, "height": 120})).unwrap()["id"].as_u64().unwrap());
    s.execute("node.set", &json!({"ids": [r], "props": {"topLeftRadius": 30, "bottomLeftRadius": 30}})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    (Ui::new(s), r)
}

/// The rectangle's radii as `[top left, top right, bottom right, bottom left]`.
fn radii(u: &Ui, r: NodeId) -> [f64; 4] {
    u.app.session.doc().unwrap().node(r).unwrap().corners.radii
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

/// Corner field `k` (an index into the radii).
fn field(u: &Ui, k: usize) -> Rect {
    u.ctx.read_response(Id::new(("corner", k)).with("field")).expect("corner field shows").rect
}

/// The fields sit at their corners, and typing in the bottom right one sets only that corner in a
/// single undo step (#1002: the bottom row showed bottom right on the left).
#[test]
fn corner_fields_sit_at_their_corners_and_set_their_own() {
    let (mut u, r) = scene();
    let t = named_button(&mut u, "Individual corners");
    click(&mut u, t);
    let (tl, tr, br, bl) = (field(&u, 0).center(), field(&u, 1).center(), field(&u, 2).center(), field(&u, 3).center());
    assert!(tl.x < tr.x && (tl.y - tr.y).abs() < 1.0, "top row: top left, then top right");
    assert!(bl.x < br.x && (bl.y - br.y).abs() < 1.0, "bottom row: bottom left, then bottom right");
    assert!(bl.y > tl.y && (bl.x - tl.x).abs() < 1.0, "bottom left sits under top left");
    let history = u.app.session.undo_depth().0;
    click(&mut u, br);
    u.frame(vec![Event::Text("12".into())]);
    enter(&mut u);
    u.frame(vec![]);
    assert_eq!(radii(&u, r), [30.0, 0.0, 12.0, 30.0]);
    assert_eq!(u.app.session.undo_depth().0, history + 1);
}
