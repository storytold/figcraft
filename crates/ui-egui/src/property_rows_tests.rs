//! A fill, stroke or effect row of the Design panel highlights on a click on its background, and
//! the keyboard copies and pastes it ("Copy and paste properties between layers": click to
//! highlight the property in the right sidebar, Cmd/Ctrl+C, select another layer, Cmd/Ctrl+V).

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

/// Cmd/Ctrl+C, as the desktop shell delivers it.
fn copy(u: &mut Ui) {
    u.frame(vec![Event::Copy]);
}

/// Cmd/Ctrl+V with the system clipboard's text, as the desktop shell delivers it.
fn paste(u: &mut Ui, clip: &Rc<RefCell<Option<String>>>) {
    let text = clip.borrow().clone().unwrap_or_default();
    u.frame(vec![Event::Paste(text)]);
}

/// Select `id` only, and draw the panel for it.
fn select(u: &mut Ui, id: NodeId) {
    u.app.run("select.set", json!({"ids": [id]}));
    u.frame(vec![]);
}

/// The hex codes of `id`'s fills, bottom to top.
fn fills(u: &Ui, id: NodeId) -> Vec<String> {
    let doc = u.app.session.doc().unwrap();
    doc.node(id).unwrap().fills.iter().map(|p| p.solid_color().unwrap().to_hex()).collect()
}

#[test]
fn a_click_on_a_fill_rows_background_highlights_it_and_the_keyboard_copies_and_pastes_it() {
    let (mut u, a, b, clip) = setup();
    click_row(&mut u, "FF0000");
    assert_eq!(
        u.app.ui.property_row.as_ref().map(|h| (h.kind, h.index, h.ids.clone())),
        Some((PropertyKind::Fill, 0, vec![a])),
        "the row is highlighted for A"
    );
    let layers_before = u.app.session.clipboard.layers_generation;
    copy(&mut u);
    let (fills_flag, paint) = u.app.session.clipboard.paint.clone().expect("the fill is on the clipboard");
    assert!(fills_flag);
    assert_eq!(paint, u.app.session.doc().unwrap().node(a).unwrap().fills[0]);
    assert_eq!(u.app.session.clipboard.layers_generation, layers_before, "the layers were not copied");
    assert!(u.app.session.clipboard.nodes.is_empty());
    assert_eq!(clip.borrow().as_deref(), Some("#FF0000"), "the colour goes to the system clipboard as its hex code");
    // Another layer selected, the paste puts the fill on top of its own.
    select(&mut u, b);
    assert!(u.app.ui.property_row.is_none(), "the highlight belonged to A");
    let layers = u.app.session.doc().unwrap().node_count();
    paste(&mut u, &clip);
    assert_eq!(fills(&u, b), ["00FF00", "FF0000"]);
    assert_eq!(u.app.session.doc().unwrap().node_count(), layers, "no layer was pasted");
    assert_eq!(fills(&u, a), ["FF0000"], "A keeps its fill");
    // One undo step takes it back off.
    u.app.run("edit.undo", json!({}));
    assert_eq!(fills(&u, b), ["00FF00"]);
}

#[test]
fn a_paste_replaces_the_highlighted_row_of_the_target() {
    let (mut u, _, b, clip) = setup();
    click_row(&mut u, "FF0000");
    copy(&mut u);
    select(&mut u, b);
    click_row(&mut u, "00FF00");
    assert_eq!(u.app.ui.property_row.as_ref().map(|h| (h.kind, h.index)), Some((PropertyKind::Fill, 0)));
    paste(&mut u, &clip);
    assert_eq!(fills(&u, b), ["FF0000"], "in place of B's highlighted fill");
}

#[test]
fn a_stroke_row_copies_its_paint_and_settings() {
    let (mut u, a, b, clip) = setup();
    click_row(&mut u, "0000FF");
    assert_eq!(u.app.ui.property_row.as_ref().map(|h| h.kind), Some(PropertyKind::Stroke));
    copy(&mut u);
    select(&mut u, b);
    paste(&mut u, &clip);
    let doc = u.app.session.doc().unwrap();
    let (na, nb) = (doc.node(a).unwrap(), doc.node(b).unwrap());
    assert_eq!(nb.strokes, na.strokes);
    assert_eq!(nb.stroke.weight, 3.0, "the weight comes with the stroke");
    assert_eq!(fills(&u, b), ["00FF00"], "the fills are untouched");
}

#[test]
fn an_effect_row_copies_and_pastes_its_effect() {
    let (mut u, a, b, clip) = setup();
    click_row(&mut u, "Layer blur");
    assert_eq!(u.app.ui.property_row.as_ref().map(|h| (h.kind, h.index)), Some((PropertyKind::Effect, 0)));
    copy(&mut u);
    assert_eq!(clip.borrow().as_deref(), Some("figcraft://clipboard/effect"), "an effect has no text: a marker of our copy");
    select(&mut u, b);
    paste(&mut u, &clip);
    let doc = u.app.session.doc().unwrap();
    let blur = doc.node(a).unwrap().effects[0].clone();
    assert_eq!(doc.node(b).unwrap().effects.len(), 2);
    assert_eq!(doc.node(b).unwrap().effects[1], blur, "the blur goes on top of B's shadow");
}

#[test]
fn without_a_highlighted_row_the_keyboard_copies_and_pastes_layers() {
    let (mut u, _, b, clip) = setup();
    copy(&mut u);
    assert!(u.app.session.clipboard.paint.is_none(), "no fill was copied");
    assert_eq!(u.app.session.clipboard.nodes.len(), 1, "A was copied");
    select(&mut u, b);
    let layers = u.app.session.doc().unwrap().node_count();
    paste(&mut u, &clip);
    assert_eq!(u.app.session.doc().unwrap().node_count(), layers + 1, "a copy of A was pasted");
    assert_eq!(fills(&u, b), ["00FF00"]);
}

#[test]
fn copying_layers_after_a_fill_pastes_the_layers() {
    let (mut u, a, b, clip) = setup();
    click_row(&mut u, "FF0000");
    copy(&mut u);
    // Another selection lets the highlight go; the next copy takes the layers.
    select(&mut u, b);
    select(&mut u, a);
    assert!(u.app.ui.property_row.is_none(), "selecting A again doesn't bring the highlight back");
    copy(&mut u);
    select(&mut u, b);
    let layers = u.app.session.doc().unwrap().node_count();
    paste(&mut u, &clip);
    assert_eq!(u.app.session.doc().unwrap().node_count(), layers + 1, "the layers were the last copy");
    assert_eq!(fills(&u, b), ["00FF00"]);
}

#[test]
fn text_copied_in_another_app_afterwards_pastes_as_text() {
    let (mut u, _, b, clip) = setup();
    click_row(&mut u, "FF0000");
    copy(&mut u);
    select(&mut u, b);
    *clip.borrow_mut() = Some("Hello".into());
    let layers = u.app.session.doc().unwrap().node_count();
    paste(&mut u, &clip);
    assert_eq!(fills(&u, b), ["00FF00"], "the fill isn't what the clipboard holds now");
    assert_eq!(u.app.session.doc().unwrap().node_count(), layers + 1, "the text became a text layer");
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
    // With nothing highlighted Cmd/Ctrl+C copies the layer again.
    copy(&mut u);
    assert!(u.app.session.clipboard.paint.is_none());
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
fn edit_copy_from_the_menu_copies_the_highlighted_row() {
    let (mut u, _, _, _) = setup();
    click_row(&mut u, "FF0000");
    crate::menus::invoke(&mut u.app, "edit.copy");
    assert!(u.app.session.clipboard.paint.as_ref().is_some_and(|(fills, _)| *fills), "the fill was copied");
    assert!(u.app.session.clipboard.nodes.is_empty(), "not the layer");
}

#[test]
fn a_paste_onto_several_layers_adds_the_fill_to_each_in_one_undo_step() {
    let (mut u, a, b, clip) = setup();
    let c = rect(&mut u.app.session, 200.0, "#0000FF");
    // A new rectangle is selected: A again.
    select(&mut u, a);
    click_row(&mut u, "FF0000");
    copy(&mut u);
    // B and C have different fills: the panel shows mixed content, no row to highlight.
    u.app.run("select.set", json!({"ids": [b, c]}));
    u.frame(vec![]);
    paste(&mut u, &clip);
    assert_eq!(fills(&u, b), ["00FF00", "FF0000"]);
    assert_eq!(fills(&u, c), ["0000FF", "FF0000"]);
    u.app.run("edit.undo", json!({}));
    assert_eq!((fills(&u, b), fills(&u, c)), (vec!["00FF00".to_string()], vec!["0000FF".to_string()]));
}

#[test]
fn a_paste_replaces_the_highlighted_row_on_every_selected_layer() {
    let (mut u, a, b, clip) = setup();
    let c = rect(&mut u.app.session, 200.0, "#00FF00");
    // A new rectangle is selected: A again.
    select(&mut u, a);
    click_row(&mut u, "FF0000");
    copy(&mut u);
    // B and C share their fill: its row shows for both.
    u.app.run("select.set", json!({"ids": [b, c]}));
    u.frame(vec![]);
    click_row(&mut u, "00FF00");
    assert_eq!(u.app.ui.property_row.as_ref().map(|h| h.ids.clone()), Some(vec![b, c]));
    paste(&mut u, &clip);
    assert_eq!((fills(&u, b), fills(&u, c)), (vec!["FF0000".to_string()], vec!["FF0000".to_string()]));
    u.app.run("edit.undo", json!({}));
    assert_eq!((fills(&u, b), fills(&u, c)), (vec!["00FF00".to_string()], vec!["00FF00".to_string()]));
}

#[test]
fn a_copied_stroke_goes_to_the_strokes_with_a_fill_row_highlighted() {
    let (mut u, a, b, clip) = setup();
    click_row(&mut u, "0000FF");
    copy(&mut u);
    select(&mut u, b);
    click_row(&mut u, "00FF00");
    assert_eq!(u.app.ui.property_row.as_ref().map(|h| h.kind), Some(PropertyKind::Fill));
    paste(&mut u, &clip);
    let doc = u.app.session.doc().unwrap();
    assert_eq!(doc.node(b).unwrap().strokes, doc.node(a).unwrap().strokes, "the stroke went to the strokes");
    assert_eq!(fills(&u, b), ["00FF00"], "the highlighted fill wasn't replaced");
}

#[test]
fn an_effect_copied_after_a_fill_is_what_pastes() {
    let (mut u, a, b, clip) = setup();
    click_row(&mut u, "FF0000");
    copy(&mut u);
    click_row(&mut u, "Layer blur");
    copy(&mut u);
    select(&mut u, b);
    paste(&mut u, &clip);
    assert_eq!(fills(&u, b), ["00FF00"], "the fill was copied first: not pasted");
    let doc = u.app.session.doc().unwrap();
    assert_eq!(doc.node(b).unwrap().effects.get(1), doc.node(a).unwrap().effects.first(), "the blur went on top of B's shadow");
}

#[test]
fn a_row_menu_copy_is_pasted_by_the_keyboard() {
    let (mut u, _, b, clip) = setup();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let p = text_center(&shapes, "FF0000");
    for pressed in [true, false] {
        u.frame(vec![Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE }]);
    }
    let menu = flat(&u.frame_output(vec![]).shapes);
    click(&mut u, text_center(&menu, crate::i18n::tr("Copy fill")));
    assert_eq!(clip.borrow().as_deref(), Some("#FF0000"));
    select(&mut u, b);
    paste(&mut u, &clip);
    assert_eq!(fills(&u, b), ["00FF00", "FF0000"]);
}

#[test]
fn programmatic_copy_and_paste_of_a_row_leave_the_system_clipboard_alone() {
    let (mut u, a, b, clip) = setup();
    *clip.borrow_mut() = Some("the user's text".into());
    let doc = u.app.session.ds().unwrap().uid;
    u.app.ui.property_row = Some(Highlight { kind: PropertyKind::Fill, index: 0, ids: vec![a], doc, rect: None });
    u.app.control_synthetic = true;
    crate::menus::invoke(&mut u.app, "edit.copy");
    assert_eq!(clip.borrow().as_deref(), Some("the user's text"), "never written");
    u.app.run("select.set", json!({"ids": [b]}));
    crate::menus::paste(&mut u.app, None, None);
    assert_eq!(fills(&u, b), ["00FF00", "FF0000"]);
}

#[test]
fn a_property_copy_whose_engine_copy_is_gone_pastes_no_stale_layers() {
    let (mut u, a, b, clip) = setup();
    // A layer copy first, then the fill row's.
    copy(&mut u);
    click_row(&mut u, "FF0000");
    copy(&mut u);
    // A stroke copied without its row (the Actions palette): the fill is no longer on the clipboard.
    u.app.run("stroke.copy", json!({"id": a}));
    select(&mut u, b);
    let layers = u.app.session.doc().unwrap().node_count();
    paste(&mut u, &clip);
    assert_eq!(u.app.session.doc().unwrap().node_count(), layers, "the layer copied before wasn't pasted");
    assert_eq!(fills(&u, b), ["00FF00"]);
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

#[test]
fn a_programmatic_paste_after_a_property_copy_whose_engine_copy_is_gone_pastes_no_stale_layers() {
    let (mut u, a, b, _) = setup();
    u.app.control_synthetic = true;
    // A layer copy first, then the fill row's, both programmatic.
    crate::menus::invoke(&mut u.app, "edit.copy");
    assert_eq!(u.app.session.clipboard.nodes.len(), 1, "A was copied");
    crate::property_rows::copy(&mut u.app, PropertyKind::Fill, 0, &[a]);
    // A stroke copied without its row: the fill is no longer on the clipboard.
    u.app.run("stroke.copy", json!({"id": a}));
    u.app.run("select.set", json!({"ids": [b]}));
    let layers = u.app.session.doc().unwrap().node_count();
    crate::menus::paste(&mut u.app, None, None);
    assert_eq!(u.app.session.doc().unwrap().node_count(), layers, "the layer copied before wasn't pasted");
    assert_eq!(fills(&u, b), ["00FF00"]);
    assert!(u.app.session.doc().unwrap().node(b).unwrap().strokes.is_empty());
}

#[test]
fn an_effect_paste_replaces_the_highlighted_effect_above_the_bottom_one() {
    let (mut u, a, b, clip) = setup();
    u.app.run("effect.add", json!({"ids": [b], "type": "innerShadow"}));
    click_row(&mut u, "Layer blur");
    copy(&mut u);
    select(&mut u, b);
    // B's effects, bottom to top: a drop shadow, an inner shadow (drawn first, at the top).
    click_row(&mut u, "Inner shadow");
    assert_eq!(u.app.ui.property_row.as_ref().map(|h| (h.kind, h.index)), Some((PropertyKind::Effect, 1)));
    paste(&mut u, &clip);
    let doc = u.app.session.doc().unwrap();
    let (blur, effects) = (&doc.node(a).unwrap().effects[0], &doc.node(b).unwrap().effects);
    assert_eq!(effects.len(), 2, "replaced, not added");
    assert_eq!(effects[1], *blur, "the blur took the inner shadow's place");
    assert_eq!(effects[0].label(), "Drop shadow", "the drop shadow under it stays");
}

#[test]
fn a_property_copied_in_one_window_pastes_in_another_and_the_highlight_stays_in_its_window() {
    let mut s = Session::empty();
    s.execute("file.new", &json!({"name": "A"})).unwrap();
    let a = rect(&mut s, 0.0, "#FF0000");
    s.execute("file.new", &json!({"name": "B"})).unwrap();
    let b = rect(&mut s, 0.0, "#00FF00");
    let clip = Rc::new(RefCell::new(None));
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = crate::FigApp::new(s, Box::new(Clip(clip.clone())));
    ctx.run_ui(egui::RawInput::default(), |ui| app.ui(ui)).textures_delta.clear();
    // File A goes to a window of its own; the main window shows B.
    let w = crate::control::handle(&mut app, &ctx, "window.new", &json!({"index": 0})).unwrap()["window"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [b]}));
    let doc = app.session.ds().unwrap().uid;
    app.ui.property_row = Some(Highlight { kind: PropertyKind::Fill, index: 0, ids: vec![b], doc, rect: None });
    crate::menus::invoke(&mut app, "edit.copy");
    assert_eq!(clip.borrow().as_deref(), Some("#00FF00"));
    let text = clip.borrow().clone();
    let pasted = crate::windows::as_window(&mut app, w, |app| {
        assert!(app.ui.property_row.is_none(), "the highlight belongs to the main window");
        app.run("select.set", json!({"ids": [a]}));
        crate::menus::paste(app, text.as_deref(), None);
        app.session.doc().unwrap().node(a).unwrap().fills.iter().map(|p| p.solid_color().unwrap().to_hex()).collect::<Vec<_>>()
    });
    let fills = pasted.expect("the window exists");
    assert_eq!(fills, ["FF0000", "00FF00"], "B's fill went on top of A's");
    assert!(app.ui.property_row.is_some(), "the main window keeps its highlight");
}
