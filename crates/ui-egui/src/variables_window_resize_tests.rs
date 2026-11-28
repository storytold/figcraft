//! The Local variables window resizes from its edges and corners, height as well as width, stops at
//! a minimum size, and keeps its size when it is closed and opened again (#2354); wider, it shows more mode columns (#2572).

use egui::{Event, Id, Pos2, Rect, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, texts};

const COUNT: usize = 60;

/// The window open on a collection of many number variables.
fn window() -> Ui {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    for k in 0..COUNT {
        s.execute("variable.create", &json!({"collection": "Theme", "name": format!("v{k:02}"), "type": "number", "value": 1})).unwrap();
    }
    let mut u = Ui::new(s);
    u.app.ui.variables = Some(Default::default());
    for _ in 0..10 {
        u.frame(vec![]);
    }
    u
}

fn rect(u: &Ui) -> Rect {
    u.ctx.memory(|m| m.area_rect(Id::unique("variables_window"))).expect("the variables window has an area")
}

/// How many variable rows are drawn inside the window (the table clips the rest).
fn rows(u: &mut Ui) -> usize {
    let window = rect(u);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    texts(&shapes).iter().filter(|(t, p, _)| t.len() == 3 && t.starts_with('v') && window.contains(*p)).count()
}

/// Drag the bottom-right corner by `by` and let go.
fn drag_corner(u: &mut Ui, by: egui::Vec2) {
    let from = rect(u).right_bottom();
    u.drag_to(from, from + by);
    let to: Pos2 = from + by;
    u.frame(vec![Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
    for _ in 0..5 {
        u.frame(vec![]);
    }
}

#[test]
fn the_variables_window_grows_when_dragged_taller() {
    let mut u = window();
    let before = rect(&u);
    let rows_before = rows(&mut u);
    drag_corner(&mut u, vec2(0.0, 200.0));
    let after = rect(&u);
    assert!((after.height() - before.height() - 200.0).abs() < 2.0, "height {} -> {}", before.height(), after.height());
    assert!((after.min.y - before.min.y).abs() < 2.0, "the top edge stays where it was: {} -> {}", before.min.y, after.min.y);
    let rows_after = rows(&mut u);
    assert!(rows_after > rows_before, "the table shows more rows: {rows_before} -> {rows_after}");
}

#[test]
fn on_a_narrow_screen_the_variables_window_fits_inside_it() {
    // Narrower than the minimum (600): the window takes the screen less its margin instead.
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    let mut u = Ui::new(s);
    u.screen = vec2(550.0, 700.0);
    u.app.ui.variables = Some(Default::default());
    for _ in 0..10 {
        u.frame(vec![]);
    }
    let r = rect(&u);
    let screen = Rect::from_min_size(Pos2::ZERO, u.screen);
    assert!(screen.contains_rect(r), "the window {r:?} is inside the screen {screen:?}");
}

#[test]
fn on_a_short_screen_the_variables_window_fits_inside_it() {
    // Lower than the minimum (240): the body takes what is left below the header and scrolls.
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    for k in 0..COUNT {
        s.execute("variable.create", &json!({"collection": "Theme", "name": format!("v{k:02}"), "type": "number", "value": 1})).unwrap();
    }
    let mut u = Ui::new(s);
    u.screen = vec2(800.0, 180.0);
    u.app.ui.variables = Some(Default::default());
    for _ in 0..10 {
        u.frame(vec![]);
    }
    let r = rect(&u);
    let screen = Rect::from_min_size(Pos2::ZERO, u.screen);
    assert!(screen.contains_rect(r), "the window {r:?} is inside the screen {screen:?}");
}

#[test]
fn the_variables_window_keeps_a_minimum_size() {
    let mut u = window();
    drag_corner(&mut u, vec2(-2000.0, -2000.0));
    let r = rect(&u);
    assert!((r.width() - 600.0).abs() < 3.0, "width {}", r.width());
    assert!((r.height() - 240.0).abs() < 3.0, "height {}", r.height());
}

#[test]
fn a_resized_variables_window_keeps_its_size_when_reopened() {
    let mut u = window();
    drag_corner(&mut u, vec2(0.0, 150.0));
    let resized = rect(&u);
    let close = resized.right_top() + vec2(-22.0, 22.0);
    u.button(close, true);
    u.button(close, false);
    for _ in 0..5 {
        u.frame(vec![]);
    }
    assert!(u.app.ui.variables.is_none(), "the Close button closed the window");
    u.app.ui.variables = Some(Default::default());
    for _ in 0..10 {
        u.frame(vec![]);
    }
    let again = rect(&u);
    assert!((again.width() - resized.width()).abs() < 1.0 && (again.height() - resized.height()).abs() < 1.0, "{resized:?} -> {again:?}");
}

/// How many mode column headings of the table are in view (painted inside their clip, which the
/// table's scroll area sets).
fn mode_columns(u: &mut Ui) -> usize {
    let window = rect(u);
    let out = u.frame_output(vec![]);
    out.shapes
        .iter()
        .filter(|c| matches!(&c.shape, egui::Shape::Text(t) if t.galley.text().starts_with("Mode ") && c.clip_rect.contains(t.pos) && window.contains(t.pos)))
        .count()
}

#[test]
fn a_wider_variables_window_shows_more_mode_columns() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "v00", "type": "number", "value": 1})).unwrap();
    for k in 2..=6 {
        s.execute("variable.addMode", &json!({"collection": "Theme", "name": format!("Mode {k}")})).unwrap();
    }
    let mut u = Ui::new(s);
    u.app.ui.variables = Some(Default::default());
    for _ in 0..10 {
        u.frame(vec![]);
    }
    let before = mode_columns(&mut u);
    let room = u.screen.x - rect(&u).max.x - 20.0;
    drag_corner(&mut u, vec2(room, 0.0));
    let after = mode_columns(&mut u);
    assert!(after > before, "the wider table shows more mode columns: {before} -> {after}");
    assert!(after < 6, "the rest of the six modes scroll into view: {after} shown");
}
