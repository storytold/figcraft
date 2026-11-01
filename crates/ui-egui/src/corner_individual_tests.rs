//! Individual corners in the Design panel, driven headless: the four corner fields sit where
//! their corners are (top left, top right / bottom left, bottom right) and each sets its own;
//! for a polygon or star the panel shows corner smoothing only.

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
    u.ctx.read_response(Id::unique(("corner", k)).with("field")).expect("corner field shows").rect
}

/// The iOS button next to Corner smoothing sets 60% in one undo step, as the help page's "click
/// iOS to set corner smoothing to 60%" describes.
#[test]
fn ios_button_sets_corner_smoothing_to_60_percent() {
    let (mut u, r) = scene();
    let t = named_button(&mut u, "Individual corners");
    click(&mut u, t);
    let history = u.app.session.undo_depth().0;
    let ios = named_button(&mut u, "iOS");
    click(&mut u, ios);
    let smoothing = u.app.session.doc().unwrap().node(r).unwrap().corners.smoothing;
    assert!((smoothing - 0.6).abs() < 1e-9, "smoothing is {smoothing}");
    assert_eq!(u.app.session.undo_depth().0, history + 1);
}

/// The visible smoothing icon uses the same scrub gesture, cursor and modifier rates as the
/// other numeric fields; the whole drag is one undo step, including when clamped at either end.
#[test]
fn corner_smoothing_icon_scrubs_with_modifiers_and_one_undo_step() {
    for (dx, modifiers, want) in
        [(16.0, Modifiers::NONE, 0.66), (8.0, Modifiers::SHIFT, 1.0), (16.0, Modifiers::ALT, 0.516), (-80.0, Modifiers::NONE, 0.0)]
    {
        let (mut u, r) = scene();
        u.app.run("node.set", json!({"ids": [r], "props": {"cornerSmoothing": 0.5}}));
        let toggle = named_button(&mut u, "Individual corners");
        click(&mut u, toggle);
        let lead = u.ctx.read_response(Id::unique("smoothing").with("lead")).expect("smoothing icon").rect;
        assert_eq!(lead.width(), 24.0, "the icon has the same drag target as other fields");
        let from = lead.center();
        let out = u.frame_output(vec![Event::PointerMoved(from)]);
        assert_eq!(out.platform_output.cursor_icon, egui::CursorIcon::ResizeHorizontal);
        let history = u.app.session.undo_depth().0;
        u.frame_with(vec![Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers }], modifiers);
        for step in 1..=4 {
            u.frame_with(vec![Event::PointerMoved(pos2(from.x + dx * step as f32 / 4.0, from.y))], modifiers);
        }
        let smoothing = |u: &Ui| u.app.session.doc().unwrap().node(r).unwrap().corners.smoothing;
        assert!((smoothing(&u) - want).abs() < 1e-9, "the drag updates the canvas before release");
        u.frame_with(
            vec![Event::PointerButton { pos: pos2(from.x + dx, from.y), button: egui::PointerButton::Primary, pressed: false, modifiers }],
            modifiers,
        );
        u.frame(vec![]);
        assert!((smoothing(&u) - want).abs() < 1e-9, "release keeps the scrubbed value");
        assert_eq!(u.app.session.undo_depth().0, history + 1);
        u.app.session.undo().unwrap();
        assert_eq!(smoothing(&u), 0.5);
        u.app.session.redo().unwrap();
        assert!((smoothing(&u) - want).abs() < 1e-9);
    }
}

/// Fine drags keep their fractional baseline across gestures, including a later plain drag.
#[test]
fn consecutive_smoothing_drags_preserve_fractional_percentages() {
    let (mut u, r) = scene();
    u.app.run("node.set", json!({"ids": [r], "props": {"cornerSmoothing": 0.5}}));
    let toggle = named_button(&mut u, "Individual corners");
    click(&mut u, toggle);
    let history = u.app.session.undo_depth().0;
    let smoothing = |u: &Ui| u.app.session.doc().unwrap().node(r).unwrap().corners.smoothing;
    for (step, (dx, modifiers, want)) in
        [(16.0, Modifiers::ALT, 0.516), (16.0, Modifiers::ALT, 0.532), (8.0, Modifiers::NONE, 0.612)].into_iter().enumerate()
    {
        let from = u.ctx.read_response(Id::unique("smoothing").with("lead")).unwrap().rect.center();
        u.frame_with(vec![Event::PointerMoved(from)], modifiers);
        u.frame_with(vec![Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers }], modifiers);
        for k in 1..=4 {
            u.frame_with(vec![Event::PointerMoved(pos2(from.x + dx * k as f32 / 4.0, from.y))], modifiers);
        }
        u.frame_with(
            vec![Event::PointerButton { pos: pos2(from.x + dx, from.y), button: egui::PointerButton::Primary, pressed: false, modifiers }],
            modifiers,
        );
        u.frame(vec![]);
        assert!((smoothing(&u) - want).abs() < 1e-9, "gesture {step}: expected {want}, got {}", smoothing(&u));
        assert_eq!(u.app.session.undo_depth().0, history + step + 1);
    }
    for want in [0.532, 0.516, 0.5] {
        u.app.session.undo().unwrap();
        assert!((smoothing(&u) - want).abs() < 1e-9, "undo restores the precise value");
    }
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

/// For a rounded polygon or star the same panel shows corner smoothing, which applies to the
/// whole shape: no field per corner, and typing a percentage sets the smoothing in one undo step.
#[test]
fn polygons_and_stars_take_corner_smoothing() {
    for cmd in ["shape.polygon", "shape.star"] {
        let mut s = Session::new();
        let id = NodeId(s.execute(cmd, &json!({"x": 0, "y": 0, "width": 120, "height": 120})).unwrap()["id"].as_u64().unwrap());
        s.execute("node.set", &json!({"ids": [id], "props": {"cornerRadius": 8}})).unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let mut u = Ui::new(s);
        // With no corners of their own, the button is named for what it opens.
        let t = named_button(&mut u, "Corner smoothing");
        click(&mut u, t);
        assert!((0..4).all(|k| u.ctx.read_response(Id::unique(("corner", k)).with("field")).is_none()), "{cmd}: one radius, no corner fields");
        let smoothing = u.ctx.read_response(Id::unique("smoothing").with("field")).expect("the smoothing field shows").rect.center();
        let history = u.app.session.undo_depth().0;
        click(&mut u, smoothing);
        u.frame(vec![Event::Text("60".into())]);
        enter(&mut u);
        u.frame(vec![]);
        let n = u.app.session.doc().unwrap().node(id).unwrap().clone();
        assert!((n.corners.smoothing - 0.6).abs() < 1e-9, "{cmd}: {}", n.corners.smoothing);
        assert_eq!(u.app.session.undo_depth().0, history + 1, "{cmd}: one undo step");
    }
}
