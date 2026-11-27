//! Variables made and named with a group shown in the Local variables window: Create variable
//! and Duplicate put the variable in that group, and renaming it in its row edits the part shown.
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center, texts};

fn window_with_groups() -> Ui {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    for n in ["space/sm", "color/brand"] {
        s.execute("variable.create", &json!({"collection": "Theme", "name": n, "type": "number", "value": 1})).unwrap();
    }
    let mut u = Ui::new(s);
    u.app.ui.variables = Some(Default::default());
    settle(&mut u);
    u
}

fn settle(u: &mut Ui) {
    for _ in 0..8 {
        u.frame(vec![]);
    }
}

fn click(u: &mut Ui, p: Pos2, button: PointerButton) {
    for pressed in [true, false] {
        u.frame(vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button, pressed, modifiers: Modifiers::NONE }]);
    }
    settle(u);
}

fn leftmost(u: &mut Ui, s: &str) -> Pos2 {
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let (_, at, size) =
        texts(&shapes).into_iter().filter(|(t, ..)| t == s).min_by(|a, b| a.1.x.total_cmp(&b.1.x)).unwrap_or_else(|| panic!("{s} drawn"));
    at + size / 2.0
}

fn names(u: &Ui) -> Vec<String> {
    let mut v: Vec<String> = u.app.session.doc().unwrap().variables.variables.iter().map(|v| v.name.clone()).collect();
    v.sort();
    v
}

fn shown(u: &mut Ui, s: &str) -> bool {
    has_text(&flat(&u.frame_output(vec![]).shapes), s)
}

#[test]
fn a_variable_created_with_a_group_shown_goes_into_that_group_and_keeps_it_when_named() {
    let mut u = window_with_groups();
    let group = leftmost(&mut u, "space");
    click(&mut u, group, PointerButton::Primary);
    assert!(!shown(&mut u, "brand"), "only the space group is shown");

    let create = text_center(&flat(&u.frame_output(vec![]).shapes), "+ Create variable");
    click(&mut u, create, PointerButton::Primary);
    let number = text_center(&flat(&u.frame_output(vec![]).shapes), "Number");
    click(&mut u, number, PointerButton::Primary);
    assert_eq!(names(&u), ["color/brand", "space/Number", "space/sm"]);

    u.frame(vec![Event::Text("lg".into())]);
    for pressed in [true, false] {
        u.frame(vec![Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }]);
    }
    settle(&mut u);
    assert_eq!(names(&u), ["color/brand", "space/lg", "space/sm"], "naming it in its row keeps the group");
    assert!(shown(&mut u, "lg"), "and it is in the table shown");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn a_variable_duplicated_in_a_group_stays_in_it_and_in_view() {
    let mut u = window_with_groups();
    let group = leftmost(&mut u, "space");
    click(&mut u, group, PointerButton::Primary);
    let row = leftmost(&mut u, "sm");
    click(&mut u, row, PointerButton::Secondary);
    let duplicate = text_center(&flat(&u.frame_output(vec![]).shapes), "Duplicate");
    click(&mut u, duplicate, PointerButton::Primary);
    assert_eq!(names(&u), ["color/brand", "space/sm", "space/sm copy"]);
    assert!(shown(&mut u, "sm copy"));
}

fn create_number(u: &mut Ui) {
    let create = text_center(&flat(&u.frame_output(vec![]).shapes), "+ Create variable");
    click(u, create, PointerButton::Primary);
    let number = text_center(&flat(&u.frame_output(vec![]).shapes), "Number");
    click(u, number, PointerButton::Primary);
}

fn type_and_enter(u: &mut Ui, typed: &str) {
    u.frame(vec![Event::Text(typed.into())]);
    for pressed in [true, false] {
        u.frame(vec![Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }]);
    }
    settle(u);
}

#[test]
fn a_name_of_slashes_alone_leaves_the_variable_in_its_group() {
    for typed in ["/", " / / "] {
        let mut u = window_with_groups();
        let group = leftmost(&mut u, "space");
        click(&mut u, group, PointerButton::Primary);
        create_number(&mut u);
        type_and_enter(&mut u, typed);
        assert_eq!(names(&u), ["color/brand", "space/Number", "space/sm"], "{typed:?}");
    }
}

#[test]
fn a_new_collection_takes_its_first_variable_outside_the_group_shown_before() {
    let mut u = window_with_groups();
    let group = leftmost(&mut u, "space");
    click(&mut u, group, PointerButton::Primary);
    let heading = leftmost(&mut u, "Collections");
    let (_, at, _) = texts(&flat(&u.frame_output(vec![]).shapes)).into_iter().find(|(t, ..)| t == "Collections").unwrap();
    click(&mut u, Pos2::new(at.x - 16.0 + 200.0 - 20.0, heading.y), PointerButton::Primary);
    assert_eq!(u.app.session.doc().unwrap().variables.collections.len(), 2, "the + by Collections made one");
    type_and_enter(&mut u, "Sizes");
    create_number(&mut u);
    assert_eq!(names(&u), ["Number", "color/brand", "space/sm"]);
}
