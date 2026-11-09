//! A fill, stroke or effect row of the Design panel highlights on a click on its background or its
//! grip ("Copy and paste properties between layers": click to highlight the property in the right
//! sidebar).

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Event, Modifiers, PointerButton, Pos2, pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::Services;
use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, text_center};
use crate::property_rows::{Highlight, PropertyKind};

/// The test machine's system clipboard: the text last written, which another app may replace.
struct Clip(Rc<RefCell<Option<String>>>);

impl Services for Clip {
    fn clipboard_set_text(&mut self, s: &str) {
        *self.0.borrow_mut() = Some(s.to_string());
    }
    fn clipboard_get_text(&mut self) -> Option<String> {
        self.0.borrow().clone()
    }
}

/// A 40 x 40 rectangle filled with `color`; its id.
fn rect(s: &mut Session, x: f64, color: &str) -> NodeId {
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("fill.set", &json!({"ids": [id], "color": color})).unwrap();
    id
}

/// Rectangles A (red, a 3 pt blue stroke and a layer blur) and B (green, a drop shadow), A
/// selected, in a window whose system clipboard the test reads and writes.
fn setup() -> (Ui, NodeId, NodeId, Rc<RefCell<Option<String>>>) {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, "#FF0000");
    let b = rect(&mut s, 100.0, "#00FF00");
    s.execute("stroke.add", &json!({"ids": [a], "paint": {"type": "solid", "color": "#0000FF"}})).unwrap();
    s.execute("node.set", &json!({"ids": [a], "props": {"strokeWeight": 3.0}})).unwrap();
    s.execute("effect.add", &json!({"ids": [a], "type": "layerBlur"})).unwrap();
    s.execute("effect.add", &json!({"ids": [b], "type": "dropShadow"})).unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let mut u = Ui::new(s);
    let clip = Rc::new(RefCell::new(None));
    u.app.services = Box::new(Clip(clip.clone()));
    u.frame(vec![]);
    (u, a, b, clip)
}

/// A primary click at `p`, and a few frames after it.
fn click(u: &mut Ui, p: Pos2) {
    u.frame(vec![Event::PointerMoved(p)]);
    for pressed in [true, false] {
        u.frame(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    }
    u.frame(vec![]);
}

/// Click the background of the Design panel row showing `label`: its left margin, where no
/// control is (the reorder grip shows there on hover).
fn click_row(u: &mut Ui, label: &str) {
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let y = text_center(&shapes, label).y;
    let panel = u.app.ui.right_panel_rect.expect("the Design panel is open");
    click(u, pos2(panel.min.x + 6.0, y));
}

/// Select `id` only, and draw the panel for it.
fn select(u: &mut Ui, id: NodeId) {
    u.app.run("select.set", json!({"ids": [id]}));
    u.frame(vec![]);
}

#[test]
fn a_press_elsewhere_lets_the_highlight_go() {
    let (mut u, a, _, _) = setup();
    click_row(&mut u, "FF0000");
    assert!(u.app.ui.property_row.is_some());
    // A press on the row's own background keeps it.
    click_row(&mut u, "FF0000");
    assert!(u.app.ui.property_row.is_some());
    // One anywhere else in the panel lets it go, the selection staying as it is.
    let shapes = flat(&u.frame_output(vec![]).shapes);
    click(&mut u, text_center(&shapes, "Effects"));
    assert_eq!(u.app.session.selection(), [a]);
    assert!(u.app.ui.property_row.is_none());
}

#[test]
fn the_highlight_follows_its_row_when_rows_move_or_go() {
    let (mut u, a, _, _) = setup();
    let doc = u.app.session.ds().unwrap().uid;
    let at = |u: &Ui| u.app.ui.property_row.as_ref().map(|h| h.index);
    u.app.ui.property_row = Some(Highlight { kind: PropertyKind::Fill, index: 1, ids: vec![a], doc, rect: None });
    crate::property_rows::follow(&mut u.app, PropertyKind::Fill, 1, Some(3));
    assert_eq!(at(&u), Some(3), "moved with its paint");
    crate::property_rows::follow(&mut u.app, PropertyKind::Stroke, 3, None);
    assert_eq!(at(&u), Some(3), "another list's change leaves it");
    crate::property_rows::follow(&mut u.app, PropertyKind::Fill, 0, None);
    assert_eq!(at(&u), Some(2), "a paint under it went");
    crate::property_rows::follow(&mut u.app, PropertyKind::Fill, 2, None);
    assert_eq!(at(&u), None, "its paint went");
}

/// A key press with no modifiers, as the desktop shell delivers it.
fn key(u: &mut Ui, key: egui::Key) {
    u.frame(vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    u.frame(vec![Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE }]);
}

#[test]
fn escape_lets_the_highlight_go_and_keeps_the_selection() {
    let (mut u, a, _, _) = setup();
    click_row(&mut u, "FF0000");
    assert!(u.app.ui.property_row.is_some());
    key(&mut u, egui::Key::Escape);
    assert!(u.app.ui.property_row.is_none(), "Escape let the highlight go");
    assert_eq!(u.app.session.selection(), [a], "and only that: A is still selected");
    // The next Escape deselects, as it does with no row highlighted.
    key(&mut u, egui::Key::Escape);
    assert!(u.app.session.selection().is_empty());
}

#[test]
fn a_click_on_the_canvas_lets_the_highlight_go() {
    let (mut u, a, _, _) = setup();
    u.app.run("view.zoomFit", json!({}));
    u.frame(vec![]);
    click_row(&mut u, "FF0000");
    assert!(u.app.ui.property_row.is_some());
    // A click on A itself: the selection stays as it is, the highlight goes.
    let on_a = u.app.to_screen(20.0, 20.0);
    assert!(u.app.canvas.rect.contains(on_a), "A is in view");
    click(&mut u, on_a);
    assert_eq!(u.app.session.selection(), [a]);
    assert!(u.app.ui.property_row.is_none());
}

#[test]
fn a_highlighted_row_whose_paint_an_undo_took_away_goes() {
    let (mut u, a, _, _) = setup();
    u.app.run("fill.add", json!({"ids": [a], "paint": {"type": "solid", "color": "#123456"}}));
    u.frame(vec![]);
    click_row(&mut u, "123456");
    assert_eq!(u.app.ui.property_row.as_ref().map(|h| h.index), Some(1));
    u.app.run("edit.undo", json!({}));
    u.frame(vec![]);
    u.frame(vec![]);
    assert!(u.app.ui.property_row.is_none(), "its row isn't drawn any more");
    assert_eq!(u.app.session.selection(), [a]);
}

#[test]
fn a_click_beside_the_rows_controls_highlights_it_too() {
    let (mut u, _, _, _) = setup();
    // The row's grip, left of the swatch, highlights it (`click_row`); so does the gap between its
    // colour field and its visibility button (the grid's 8 pt gutter, 60 pt from the row's right).
    click_row(&mut u, "FF0000");
    let row = u.app.ui.property_row.as_ref().and_then(|h| h.rect).expect("the highlighted row was drawn");
    key(&mut u, egui::Key::Escape);
    assert!(u.app.ui.property_row.is_none());
    click(&mut u, pos2(row.max.x - 64.0, row.center().y));
    assert_eq!(u.app.ui.property_row.as_ref().map(|h| (h.kind, h.index)), Some((PropertyKind::Fill, 0)));
}

#[test]
fn a_click_on_a_rows_grip_highlights_that_row_for_the_selection() {
    let (mut u, a, b, _) = setup();
    for (label, kind) in [("FF0000", PropertyKind::Fill), ("0000FF", PropertyKind::Stroke), ("Layer blur", PropertyKind::Effect)] {
        click_row(&mut u, label);
        assert_eq!(
            u.app.ui.property_row.as_ref().map(|h| (h.kind, h.index, h.ids.clone())),
            Some((kind, 0, vec![a])),
            "{label}: one row highlighted, for A"
        );
    }
    // The row is drawn in the selected-row colour.
    let t = crate::theme::Tokens::get(&u.ctx);
    assert_eq!(crate::property_rows::fill(&u.app, PropertyKind::Effect, 0, t.selected_row), Some(t.selected_row));
    assert_eq!(crate::property_rows::fill(&u.app, PropertyKind::Fill, 0, t.selected_row), None);
    // It belonged to A: another selection lets it go, and A selected again doesn't bring it back.
    select(&mut u, b);
    assert!(u.app.ui.property_row.is_none());
    select(&mut u, a);
    assert!(u.app.ui.property_row.is_none());
}
