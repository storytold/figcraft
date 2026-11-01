//! Dragging fill, effect and layout grid rows in the Design panel, driven through egui headless:
//! a drop reorders the list as one undo step, Escape cancels, and lists that differ across the
//! selection ("Mixed") have nothing to drag.

use egui::{Event, Pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::list_drag::grip_id;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A selected rectangle with three solid fills (bottom to top: red, green, blue).
fn setup() -> (Ui, NodeId) {
    let mut s = Session::new();
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap());
    s.execute("fill.set", &json!({"ids": [r], "color": "#FF0000"})).unwrap();
    s.execute("fill.add", &json!({"ids": [r], "color": "#00FF00"})).unwrap();
    s.execute("fill.add", &json!({"ids": [r], "color": "#0000FF"})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    (Ui::new(s), r)
}

fn fills(u: &Ui, r: NodeId) -> Vec<String> {
    u.app.session.doc().unwrap().node(r).unwrap().fills.iter().filter_map(|p| p.solid_color()).map(|c| c.to_hex()).collect()
}

/// The middle of item `index`'s grip in list `list` (as laid out last frame).
fn grip(u: &Ui, list: &str, index: usize) -> Option<Pos2> {
    u.ctx.read_response(grip_id(list, index)).map(|r| r.rect.center())
}

/// Press at `from`, move to `to` in small steps, and leave the button down.
fn drag(u: &mut Ui, from: Pos2, to: Pos2) {
    u.button(from, true);
    for k in 1..=10 {
        u.frame(vec![Event::PointerMoved(from + (to - from) * (k as f32 / 10.0))]);
    }
}

#[test]
fn dragging_a_fill_row_reorders_the_fills_in_one_step() {
    let (mut u, r) = setup();
    let (bottom, top) = (grip(&u, "fills", 0).unwrap(), grip(&u, "fills", 2).unwrap());
    assert!(top.y < bottom.y, "the topmost paint is listed first");
    let undo = u.app.session.ds().unwrap().history.undo.len();
    // The bottom paint (red) above the top row: it becomes the topmost.
    drag(&mut u, bottom, top - egui::vec2(0.0, 12.0));
    assert_eq!(fills(&u, r), ["FF0000", "00FF00", "0000FF"], "nothing moves before the drop");
    u.button(top - egui::vec2(0.0, 12.0), false);
    assert_eq!(fills(&u, r), ["00FF00", "0000FF", "FF0000"]);
    assert_eq!(u.app.session.ds().unwrap().history.undo.len(), undo + 1);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    // Dropping a row where it already is changes nothing.
    let g = grip(&u, "fills", 1).unwrap();
    drag(&mut u, g, g + egui::vec2(0.0, 6.0));
    u.button(g + egui::vec2(0.0, 6.0), false);
    assert_eq!(u.app.session.ds().unwrap().history.undo.len(), undo + 1);
}

#[test]
fn escape_cancels_a_row_drag() {
    let (mut u, r) = setup();
    let (bottom, top) = (grip(&u, "fills", 0).unwrap(), grip(&u, "fills", 2).unwrap());
    drag(&mut u, bottom, top);
    u.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    u.button(top, false);
    assert_eq!(fills(&u, r), ["FF0000", "00FF00", "0000FF"]);
    assert_eq!(u.app.session.ds().unwrap().selection, [r], "the Escape only cancels the drag");
}

/// Asserts effect and layout-grid rows reorder by drag, while a mixed fill list exposes no
/// drag handle.
#[test]
fn effects_and_grids_drag_too_and_mixed_lists_dont() {
    let (mut u, r) = setup();
    let f = id(u.app.session.execute("frame.create", &json!({"x": 100, "y": 0, "width": 100, "height": 100})).unwrap());
    for p in [json!({"type": "dropShadow"}), json!({"type": "layerBlur"})] {
        u.app.session.execute("effect.add", &json!({"ids": [f], "type": p["type"]})).unwrap();
    }
    u.app.session.execute("grid.add", &json!({"ids": [f], "grid": {"kind": "columns"}})).unwrap();
    u.app.session.execute("grid.add", &json!({"ids": [f], "grid": {"kind": "rows"}})).unwrap();
    u.app.session.execute("select.set", &json!({"ids": [f]})).unwrap();
    for _ in 0..2 {
        u.frame(vec![]);
    }
    let (a, b) = (grip(&u, "effects", 0).unwrap(), grip(&u, "effects", 1).unwrap());
    drag(&mut u, a, b - egui::vec2(0.0, 12.0));
    u.button(b - egui::vec2(0.0, 12.0), false);
    let kinds: Vec<&str> = u.app.session.doc().unwrap().node(f).unwrap().effects.iter().map(|e| e.label()).collect();
    assert_eq!(kinds, ["Layer blur", "Drop shadow"]);
    // The layout grid section is below the fold: scroll the panel.
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        source: egui::MouseWheelSource::Unknown,
        delta: egui::vec2(0.0, -400.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    };
    for _ in 0..4 {
        u.frame(vec![Event::PointerMoved(Pos2::new(1300.0, 500.0)), wheel.clone()]);
    }
    u.frame(vec![]);
    let (a, b) = (grip(&u, "grids", 0).unwrap(), grip(&u, "grids", 1).unwrap());
    assert!(b.y < 880.0, "{b:?}");
    drag(&mut u, a, b - egui::vec2(0.0, 12.0));
    u.button(b - egui::vec2(0.0, 12.0), false);
    let grids: Vec<figcraft_doc::GridKind> = u.app.session.doc().unwrap().node(f).unwrap().grids.iter().map(|g| g.kind).collect();
    assert_eq!(grids, [figcraft_doc::GridKind::Rows, figcraft_doc::GridKind::Columns]);
    // The frame and the rectangle have different fills: nothing to drag.
    u.app.session.execute("select.set", &json!({"ids": [r, f]})).unwrap();
    for _ in 0..2 {
        u.frame(vec![]);
    }
    assert!(grip(&u, "fills", 0).is_none());
}
