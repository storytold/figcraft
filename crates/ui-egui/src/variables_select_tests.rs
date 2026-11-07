//! Selecting several variables in the Local variables window (#2002): a click selects one row,
//! Shift+click the range from the anchor, Ctrl/Cmd+click toggles a row, Escape and a click on the
//! empty space clear; delete, move to a group and scope then act on the whole selection in one
//! undo step.

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center};

const NAMES: [&str; 5] = ["alpha", "bravo", "charlie", "delta", "echo"];

/// The window open on a collection of five number variables.
fn window() -> Ui {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    for n in NAMES {
        s.execute("variable.create", &json!({"collection": "Theme", "name": n, "type": "number", "value": 1})).unwrap();
    }
    let mut u = Ui::new(s);
    u.app.ui.variables = Some(Default::default());
    for _ in 0..10 {
        u.frame(vec![]);
    }
    u
}

fn row(u: &mut Ui, name: &str) -> Pos2 {
    let shapes = flat(&u.frame_output(vec![]).shapes);
    // The name column, left of the value cells.
    let p = text_center(&shapes, name);
    Pos2::new(p.x, p.y)
}

fn click(u: &mut Ui, p: Pos2, button: PointerButton, m: Modifiers) {
    u.frame_with(vec![Event::PointerMoved(p)], m);
    for pressed in [true, false] {
        u.frame_with(vec![Event::PointerButton { pos: p, button, pressed, modifiers: m }], m);
    }
    for _ in 0..5 {
        u.frame(vec![]);
    }
}

fn click_row(u: &mut Ui, name: &str, m: Modifiers) {
    let p = row(u, name);
    click(u, p, PointerButton::Primary, m);
}

fn selected(u: &Ui) -> Vec<String> {
    let st = u.app.ui.variables.as_ref().unwrap();
    let doc = u.app.session.doc().unwrap();
    let mut names: Vec<String> = st.selected.iter().filter_map(|id| doc.variables.get_by_id(id)).map(|v| v.name.clone()).collect();
    names.sort();
    names
}

fn names(u: &Ui) -> Vec<String> {
    u.app.session.doc().unwrap().variables.variables.iter().map(|v| v.name.clone()).collect()
}

const SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::NONE };
const CTRL: Modifiers = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };

#[test]
fn a_click_selects_one_variable_and_replaces_the_selection() {
    let mut u = window();
    click_row(&mut u, "bravo", Modifiers::NONE);
    assert_eq!(selected(&u), ["bravo"]);
    click_row(&mut u, "delta", Modifiers::NONE);
    assert_eq!(selected(&u), ["delta"]);
}

#[test]
fn shift_click_selects_the_range_from_the_anchor() {
    let mut u = window();
    click_row(&mut u, "bravo", Modifiers::NONE);
    click_row(&mut u, "delta", SHIFT);
    assert_eq!(selected(&u), ["bravo", "charlie", "delta"]);
    // A range upward from the same anchor.
    click_row(&mut u, "alpha", SHIFT);
    assert_eq!(selected(&u), ["alpha", "bravo"]);
}

#[test]
fn ctrl_click_toggles_one_variable() {
    let mut u = window();
    click_row(&mut u, "alpha", Modifiers::NONE);
    click_row(&mut u, "charlie", CTRL);
    assert_eq!(selected(&u), ["alpha", "charlie"]);
    click_row(&mut u, "alpha", CTRL);
    assert_eq!(selected(&u), ["charlie"]);
}

#[test]
fn escape_clears_the_selection() {
    let mut u = window();
    click_row(&mut u, "alpha", Modifiers::NONE);
    click_row(&mut u, "bravo", SHIFT);
    assert_eq!(selected(&u).len(), 2);
    for pressed in [true, false] {
        u.frame(vec![Event::Key { key: Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }]);
    }
    assert!(selected(&u).is_empty());
}

#[test]
fn selected_rows_are_painted_with_the_selected_row_colour() {
    let mut u = window();
    let colour = crate::theme::Tokens::get(&u.ctx).selected_row;
    let count = |u: &mut Ui| flat(&u.frame_output(vec![]).shapes).iter().filter(|s| matches!(s, egui::Shape::Rect(r) if r.fill == colour)).count();
    let none = count(&mut u);
    click_row(&mut u, "alpha", Modifiers::NONE);
    let one = count(&mut u);
    click_row(&mut u, "charlie", SHIFT);
    let three = count(&mut u);
    assert!(one > none, "the selected row is highlighted");
    assert_eq!(three - one, 2, "two more rows highlighted by the range");
}

/// Select bravo..delta, then right-click delta.
fn select_three_and_open_menu(u: &mut Ui) -> Vec<egui::Shape> {
    click_row(u, "bravo", Modifiers::NONE);
    click_row(u, "delta", SHIFT);
    assert_eq!(selected(u), ["bravo", "charlie", "delta"]);
    let p = row(u, "delta");
    click(u, p, PointerButton::Secondary, Modifiers::NONE);
    flat(&u.frame_output(vec![]).shapes)
}

#[test]
fn bulk_delete_removes_exactly_the_selected_variables_in_one_undo_step() {
    let mut u = window();
    let menu = select_three_and_open_menu(&mut u);
    assert!(has_text(&menu, "Delete 3 variables"));
    let before = u.app.session.undo_labels(100).len();
    click(&mut u, text_center(&menu, "Delete 3 variables"), PointerButton::Primary, Modifiers::NONE);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    assert_eq!(names(&u), ["alpha", "echo"]);
    assert_eq!(u.app.session.undo_labels(100).len(), before + 1, "one undo step");
    u.app.session.undo().unwrap();
    assert_eq!(names(&u), NAMES);
}

#[test]
fn bulk_move_to_a_group_renames_every_selected_variable_in_one_step() {
    let mut u = window();
    u.app.session.execute("variable.create", &json!({"collection": "Theme", "name": "space/zero", "type": "number", "value": 0})).unwrap();
    let menu = select_three_and_open_menu(&mut u);
    let before = u.app.session.undo_labels(100).len();
    click(&mut u, text_center(&menu, "Move to group space"), PointerButton::Primary, Modifiers::NONE);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    let mut now = names(&u);
    now.sort();
    assert_eq!(now, ["alpha", "echo", "space/bravo", "space/charlie", "space/delta", "space/zero"]);
    assert_eq!(u.app.session.undo_labels(100).len(), before + 1, "one undo step");
    u.app.session.undo().unwrap();
    assert!(names(&u).contains(&"bravo".to_string()) && names(&u).contains(&"delta".to_string()));
}

#[test]
fn bulk_scope_change_reaches_every_selected_variable_in_one_step() {
    let mut u = window();
    let menu = select_three_and_open_menu(&mut u);
    let before = u.app.session.undo_labels(100).len();
    click(&mut u, text_center(&menu, "Scope: only Corner radius"), PointerButton::Primary, Modifiers::NONE);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    let scoped: Vec<String> = u
        .app
        .session
        .doc()
        .unwrap()
        .variables
        .variables
        .iter()
        .filter(|v| v.scopes.as_deref() == Some(&["CORNER_RADIUS".to_string()][..]))
        .map(|v| v.name.clone())
        .collect();
    assert_eq!(scoped, ["bravo", "charlie", "delta"]);
    assert_eq!(u.app.session.undo_labels(100).len(), before + 1, "one undo step");
}

#[test]
fn a_row_outside_the_selection_keeps_the_single_variable_menu() {
    let mut u = window();
    click_row(&mut u, "alpha", Modifiers::NONE);
    click_row(&mut u, "bravo", SHIFT);
    let p = row(&mut u, "echo");
    click(&mut u, p, PointerButton::Secondary, Modifiers::NONE);
    let menu = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&menu, "Delete variable") && !has_text(&menu, "Delete 2 variables"));
}

/// The bounding rect of every popup layer (the menu's area).
fn popup_rect(u: &Ui) -> egui::Rect {
    u.ctx.memory(|m| {
        let rects: Vec<egui::Rect> =
            m.areas().visible_layer_ids().into_iter().filter(|l| l.order == egui::Order::Foreground).filter_map(|l| m.area_rect(l.id)).collect();
        assert!(!rects.is_empty(), "no popup open");
        rects.into_iter().reduce(|a, b| a.union(b)).unwrap()
    })
}

#[test]
fn the_bulk_menu_scrolls_when_forty_groups_make_it_taller_than_the_window() {
    let mut u = window();
    for g in 0..40 {
        u.app
            .session
            .execute("variable.create", &json!({"collection": "Theme", "name": format!("g{g:02}/leaf"), "type": "number", "value": 0}))
            .unwrap();
    }
    u.screen = egui::vec2(1000.0, 500.0);
    for _ in 0..5 {
        u.frame(vec![]);
    }
    click_row(&mut u, "bravo", Modifiers::NONE);
    click_row(&mut u, "charlie", SHIFT);
    let p = row(&mut u, "charlie");
    click(&mut u, p, PointerButton::Secondary, Modifiers::NONE);
    let menu = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&menu, "Delete 2 variables"));
    let screen = egui::Rect::from_min_size(Pos2::ZERO, u.screen);
    let area = popup_rect(&u);
    assert!(screen.contains_rect(area), "the menu area {area:?} runs past the screen {screen:?}");
    let last = "Move to group g39";
    let screen_h = u.screen.y;
    let visible = |shapes: &[egui::Shape]| crate::live_tests::texts(shapes).iter().any(|(t, p, s)| t == last && p.y + s.y <= screen_h);
    assert!(!visible(&menu), "the last group row should start out of view");
    let at = Pos2::new(area.center().x, area.center().y);
    let mut shapes = menu;
    for _ in 0..30 {
        u.frame(vec![
            Event::PointerMoved(at),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                source: egui::MouseWheelSource::Unknown,
                phase: egui::TouchPhase::Move,
                delta: egui::vec2(0.0, -400.0),
                modifiers: Modifiers::NONE,
            },
        ]);
        shapes = flat(&u.frame_output(vec![Event::PointerMoved(at)]).shapes);
        if visible(&shapes) {
            break;
        }
    }
    assert!(visible(&shapes), "scrolling the menu never brought {last:?} into view");
    assert!(screen.contains_rect(popup_rect(&u)));
}
