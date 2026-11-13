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

/// A collection with a group `space` of two number variables with distinct values in two modes.
fn grouped_window() -> Ui {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap();
    for (n, light, dark) in [("space/sm", 4, 8), ("space/md", 16, 32)] {
        s.execute("variable.create", &json!({"collection": "Theme", "name": n, "type": "number", "value": light})).unwrap();
        s.execute("variable.set", &json!({"variable": n, "mode": "Dark", "value": dark})).unwrap();
        s.execute("variable.update", &json!({"variable": n, "scopes": ["GAP"], "description": n})).unwrap();
    }
    let mut u = Ui::new(s);
    u.app.ui.variables = Some(Default::default());
    for _ in 0..10 {
        u.frame(vec![]);
    }
    u
}

/// Three colour variables: bravo and charlie also have Stroke in their scopes, delta does not.
fn colour_window() -> Ui {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    for (n, scopes) in
        [("bravo", json!(["FRAME_FILL", "STROKE_COLOR"])), ("charlie", json!(["FRAME_FILL", "STROKE_COLOR"])), ("delta", json!(["FRAME_FILL"]))]
    {
        s.execute("variable.create", &json!({"collection": "Theme", "name": n, "type": "color", "value": "#ff0000"})).unwrap();
        s.execute("variable.update", &json!({"variable": n, "scopes": scopes})).unwrap();
    }
    let mut u = Ui::new(s);
    u.app.ui.variables = Some(Default::default());
    for _ in 0..10 {
        u.frame(vec![]);
    }
    u
}

/// Right-click the group row `name` and choose Duplicate group.
fn duplicate_group(u: &mut Ui, name: &str) {
    let p = row(u, name);
    click(u, p, PointerButton::Secondary, Modifiers::NONE);
    let menu = flat(&u.frame_output(vec![]).shapes);
    click(u, text_center(&menu, "Duplicate group"), PointerButton::Primary, Modifiers::NONE);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn duplicate_group_copies_every_variable_in_both_modes_in_one_undo_step() {
    let mut u = grouped_window();
    let before = u.app.session.undo_labels(100).len();
    duplicate_group(&mut u, "space");
    let doc = u.app.session.doc().unwrap();
    let vars = &doc.variables;
    for (leaf, light, dark) in [("sm", 4, 8), ("md", 16, 32)] {
        let old = vars.variables.iter().find(|v| v.name == format!("space/{leaf}")).unwrap();
        let new = vars.variables.iter().find(|v| v.name == format!("space 2/{leaf}")).unwrap_or_else(|| panic!("no space 2/{leaf}"));
        assert_ne!(old.id, new.id);
        assert_eq!(new.values, old.values, "{leaf}: values in both modes");
        assert_eq!(new.scopes, old.scopes);
        assert_eq!(new.description, old.description);
        let _ = (light, dark);
    }
    assert_eq!(names(&u).len(), 4);
    assert_eq!(u.app.session.undo_labels(100).len(), before + 1, "one undo step");
    assert_eq!(u.app.session.undo_labels(1), ["Duplicate group"]);
    // The new group is listed directly after the original.
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&shapes, "space 2"));
    assert!(text_center(&shapes, "space 2").y > text_center(&shapes, "space").y);
    u.app.session.undo().unwrap();
    assert_eq!(names(&u), ["space/sm", "space/md"]);
}

#[test]
fn a_second_duplicate_group_takes_the_next_free_name() {
    let mut u = grouped_window();
    duplicate_group(&mut u, "space");
    duplicate_group(&mut u, "space");
    let n = names(&u);
    assert!(n.contains(&"space 2/sm".to_string()) && n.contains(&"space 3/md".to_string()), "{n:?}");
    assert_eq!(n.len(), 6);
}

/// A collection with the variables `names` (numbers).
fn named_window(names: &[&str]) -> Ui {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    for n in names {
        s.execute("variable.create", &json!({"collection": "Theme", "name": n, "type": "number", "value": 1})).unwrap();
    }
    let mut u = Ui::new(s);
    u.app.ui.variables = Some(Default::default());
    for _ in 0..10 {
        u.frame(vec![]);
    }
    u
}

/// Right-click the group row `name`, choose Rename group, type `to` and press Enter.
fn rename_group(u: &mut Ui, name: &str, to: &str) {
    let p = row(u, name);
    click(u, p, PointerButton::Secondary, Modifiers::NONE);
    let menu = flat(&u.frame_output(vec![]).shapes);
    click(u, text_center(&menu, "Rename group"), PointerButton::Primary, Modifiers::NONE);
    u.frame(vec![Event::Text(to.into())]);
    for pressed in [true, false] {
        u.frame(vec![Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }]);
    }
    for _ in 0..5 {
        u.frame(vec![]);
    }
}

fn sorted_names(u: &Ui) -> Vec<String> {
    let mut n = names(u);
    n.sort();
    n
}

#[test]
fn renaming_a_group_with_a_slash_nests_it_in_a_new_parent_group_in_one_undo_step() {
    let mut u = named_window(&["Orange/100", "Orange/200", "Blue/100"]);
    let before = u.app.session.undo_labels(100).len();
    rename_group(&mut u, "Orange", "Palette/Orange");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    assert_eq!(sorted_names(&u), ["Blue/100", "Palette/Orange/100", "Palette/Orange/200"]);
    assert_eq!(u.app.session.undo_labels(100).len(), before + 1, "one undo step");
    assert_eq!(u.app.session.undo_labels(1), ["Rename group"]);
    // The sidebar lists Palette with Orange indented under it.
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let (parent, child) = (text_center(&shapes, "Palette"), text_center(&shapes, "Orange"));
    assert!(child.y > parent.y && child.x > parent.x, "Orange is indented under Palette: {parent:?} {child:?}");
    assert!(!has_text(&shapes, "Palette-Orange"));
    u.app.session.undo().unwrap();
    assert_eq!(sorted_names(&u), ["Blue/100", "Orange/100", "Orange/200"]);
}

#[test]
fn a_nested_groups_new_path_stays_in_its_parent_and_drops_empty_parts() {
    let mut u = named_window(&["Palette/Orange/100", "Palette/Orange/200"]);
    rename_group(&mut u, "Orange", " /Warm // Orange / ");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    assert_eq!(sorted_names(&u), ["Palette/Warm/Orange/100", "Palette/Warm/Orange/200"]);
}

#[test]
fn renaming_a_group_into_its_own_new_subgroup_moves_the_deeper_names_first() {
    // Orange/100 takes Orange/Warm/100, which itself moves on to Orange/Warm/Warm/100.
    let mut u = named_window(&["Orange/100", "Orange/Warm/100"]);
    rename_group(&mut u, "Orange", "Orange/Warm");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    assert_eq!(sorted_names(&u), ["Orange/Warm/100", "Orange/Warm/Warm/100"]);
}

#[test]
fn a_group_rename_onto_a_variable_name_that_stays_changes_nothing() {
    let mut u = named_window(&["Orange/100", "Orange/200", "Palette/Orange/100"]);
    rename_group(&mut u, "Orange", "Palette/Orange");
    assert!(u.app.ui.last_error.as_deref().is_some_and(|e| e.contains("Palette/Orange/100")), "{:?}", u.app.ui.last_error);
    assert_eq!(sorted_names(&u), ["Orange/100", "Orange/200", "Palette/Orange/100"]);
}

fn open_edit_variables(u: &mut Ui) -> Vec<egui::Shape> {
    let menu = select_three_and_open_menu(u);
    assert!(has_text(&menu, "Edit variables"), "the bulk menu offers Edit variables");
    click(u, text_center(&menu, "Edit variables"), PointerButton::Primary, Modifiers::NONE);
    flat(&u.frame_output(vec![]).shapes)
}

fn flags(u: &Ui) -> Vec<(bool, bool)> {
    let doc = u.app.session.doc().unwrap();
    ["bravo", "charlie", "delta"]
        .iter()
        .map(|n| {
            let v = doc.variables.variables.iter().find(|v| v.name == *n).unwrap();
            (v.hidden_from_publishing, v.in_scope("STROKE_COLOR"))
        })
        .collect()
}

#[test]
fn edit_variables_sets_stroke_on_all_selected_in_one_undo_step() {
    let mut u = colour_window();
    let shapes = open_edit_variables(&mut u);
    assert!(has_text(&shapes, "Hide from publishing") && has_text(&shapes, "Scope"));
    click(&mut u, text_center(&shapes, "Scope"), PointerButton::Primary, Modifiers::NONE);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&shapes, "Stroke"));
    assert_eq!(flags(&u), [(false, true), (false, true), (false, false)]);
    let before = u.app.session.undo_labels(100).len();
    click(&mut u, text_center(&shapes, "Stroke"), PointerButton::Primary, Modifiers::NONE);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    assert_eq!(flags(&u), [(false, true), (false, true), (false, true)]);
    assert_eq!(u.app.session.undo_labels(100).len(), before + 1, "one undo step");
    u.app.session.undo().unwrap();
    assert_eq!(flags(&u), [(false, true), (false, true), (false, false)]);
}

#[test]
fn edit_variables_hides_all_selected_from_publishing_in_one_undo_step() {
    let mut u = colour_window();
    let shapes = open_edit_variables(&mut u);
    let before = u.app.session.undo_labels(100).len();
    click(&mut u, text_center(&shapes, "Hide from publishing"), PointerButton::Primary, Modifiers::NONE);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    assert!(flags(&u).iter().all(|f| f.0), "all hidden");
    assert_eq!(u.app.session.undo_labels(100).len(), before + 1, "one undo step");
    u.app.session.undo().unwrap();
    assert!(flags(&u).iter().all(|f| !f.0));
}

/// Bravo, charlie and delta of the given types (a value fitting each).
fn typed_window(types: [&str; 3]) -> Ui {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    for (n, t) in ["bravo", "charlie", "delta"].into_iter().zip(types) {
        let value = match t {
            "number" => json!(1),
            "boolean" => json!(true),
            "string" => json!("x"),
            _ => json!("#ff0000"),
        };
        s.execute("variable.create", &json!({"collection": "Theme", "name": n, "type": t, "value": value})).unwrap();
    }
    let mut u = Ui::new(s);
    u.app.ui.variables = Some(Default::default());
    for _ in 0..10 {
        u.frame(vec![]);
    }
    u
}

fn scope_tab(u: &mut Ui) -> Vec<egui::Shape> {
    let shapes = open_edit_variables(u);
    click(u, text_center(&shapes, "Scope"), PointerButton::Primary, Modifiers::NONE);
    flat(&u.frame_output(vec![]).shapes)
}

fn scopes(u: &Ui) -> Vec<Option<Vec<String>>> {
    let doc = u.app.session.doc().unwrap();
    ["bravo", "charlie", "delta"].iter().map(|n| doc.variables.variables.iter().find(|v| v.name == *n).unwrap().scopes.clone()).collect()
}

#[test]
fn all_supported_properties_turns_on_for_every_selected_variable_and_off_again_one_undo_step_each() {
    let mut u = colour_window();
    let shapes = scope_tab(&mut u);
    assert!(scopes(&u).iter().all(|s| s.is_some()), "each variable starts limited");
    let before = u.app.session.undo_labels(100).len();
    click(&mut u, text_center(&shapes, "All supported properties"), PointerButton::Primary, Modifiers::NONE);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    assert!(scopes(&u).iter().all(|s| s.is_none()), "on: no variable is limited: {:?}", scopes(&u));
    assert_eq!(u.app.session.undo_labels(100).len(), before + 1, "one undo step");
    let shapes = flat(&u.frame_output(vec![]).shapes);
    click(&mut u, text_center(&shapes, "All supported properties"), PointerButton::Primary, Modifiers::NONE);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    assert!(scopes(&u).iter().all(|s| s.as_ref().is_some_and(|l| l.is_empty())), "off: hidden from pickers: {:?}", scopes(&u));
    assert_eq!(u.app.session.undo_labels(100).len(), before + 2, "one undo step each");
}

#[test]
fn clicking_a_checked_scope_clears_it_on_every_selected_variable_in_one_undo_step() {
    let mut u = colour_window();
    let shapes = scope_tab(&mut u);
    let before = u.app.session.undo_labels(100).len();
    // Frame fill is on for all three: checked, so the click clears it.
    click(&mut u, text_center(&shapes, "Frame fill"), PointerButton::Primary, Modifiers::NONE);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    let doc = u.app.session.doc().unwrap();
    assert!(["bravo", "charlie", "delta"].iter().all(|n| !doc.variables.variables.iter().find(|v| v.name == *n).unwrap().in_scope("FRAME_FILL")));
    assert_eq!(flags(&u), [(false, true), (false, true), (false, false)], "the other scopes stay");
    assert_eq!(u.app.session.undo_labels(100).len(), before + 1, "one undo step");
    u.app.session.undo().unwrap();
    assert!(scopes(&u).iter().all(|s| s.as_ref().is_some_and(|l| l.iter().any(|x| x == "FRAME_FILL"))));
}

#[test]
fn booleans_share_no_scope_and_the_scope_tab_says_so() {
    let mut u = typed_window(["boolean", "boolean", "boolean"]);
    let shapes = scope_tab(&mut u);
    assert!(has_text(&shapes, "No scope is shared by all the selected variables"));
    assert!(!has_text(&shapes, "All supported properties"));
}

#[test]
fn mixed_types_list_only_the_scopes_every_type_has() {
    // Number and string share only Text content; a colour shares nothing with them.
    let mut u = typed_window(["number", "number", "string"]);
    let shapes = scope_tab(&mut u);
    assert!(has_text(&shapes, "Text content"), "the shared scope is shown");
    for other in ["Corner radius", "Font family", "Font size"] {
        assert!(!has_text(&shapes, other), "{other} is not common");
    }
    assert!(!has_text(&shapes, "No scope is shared by all the selected variables"));
    let mut u = typed_window(["number", "color", "string"]);
    let shapes = scope_tab(&mut u);
    assert!(has_text(&shapes, "No scope is shared by all the selected variables"));
}

#[test]
fn edit_variables_is_not_offered_when_every_selected_variable_is_inherited() {
    let mut u = window();
    u.app.session.execute("variable.extendCollection", &json!({"collection": "Theme"})).unwrap();
    u.app.ui.variables = Some(crate::dialogs::variables_panel::Editor::on(Some("Theme (extended)".into())));
    for _ in 0..10 {
        u.frame(vec![]);
    }
    click_row(&mut u, "bravo", Modifiers::NONE);
    click_row(&mut u, "delta", SHIFT);
    assert_eq!(selected(&u), ["bravo", "charlie", "delta"]);
    let p = row(&mut u, "delta");
    click(&mut u, p, PointerButton::Secondary, Modifiers::NONE);
    let menu = flat(&u.frame_output(vec![]).shapes);
    assert!(!has_text(&menu, "Edit variables"), "inherited variables are edited in their own collection");
}
