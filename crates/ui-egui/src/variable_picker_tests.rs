//! The variable picker ("Select variable" on every bindable field, aliases, styles and prototype
//! actions) drawn headless: its list follows its rows while it is open — a search that narrows it
//! shrinks the picker, clearing the search grows it back — and it stays on the screen (#2043: egui
//! offers an area the size it had last frame, so the list used to stay a few rows high).

use egui::{Id, Pos2, RawInput, Rect, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

/// A selected rectangle and 30 number variables.
fn scene() -> (Ui, NodeId) {
    let mut s = Session::new();
    let r = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
    s.execute("variable.createCollection", &json!({"name": "Sizes"})).unwrap();
    for i in 0..30 {
        s.execute("variable.create", &json!({"collection": "Sizes", "name": format!("w-{}", 100 + 10 * i), "type": "number", "value": i})).unwrap();
    }
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    (Ui::new(s), r)
}

/// Opens the picker for `r`'s stroke weight under `at`.
fn open(u: &mut Ui, r: NodeId, at: Pos2) {
    let screen = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), ..Default::default() };
    let _ = u.ctx.run_ui(screen, |ui| crate::variable_ui::open(ui, &[r], "strokeWeight", figcraft_doc::VariableType::Number, at));
}

/// The picker's rect after a few frames.
fn settled(u: &mut Ui) -> Rect {
    for _ in 0..5 {
        u.frame(vec![]);
    }
    crate::variable_ui::picker_rect(&u.ctx).expect("the variable picker shows")
}

fn search(u: &mut Ui, q: &str) {
    u.ctx.data_mut(|d| d.insert_temp(Id::unique("variable_picker_query"), q.to_string()));
}

#[test]
fn the_variable_list_grows_back_after_a_search_narrows_it() {
    let (mut u, r) = scene();
    open(&mut u, r, pos2(900.0, 200.0));
    let open = settled(&mut u);
    assert!(open.height() > 300.0, "the picker lists the variables: {open:?}");
    search(&mut u, "w-100");
    let narrowed = settled(&mut u);
    assert!(narrowed.height() < open.height() - 100.0, "a search for one variable shrinks it: {open:?} → {narrowed:?}");
    search(&mut u, "");
    let again = settled(&mut u);
    assert!((again.height() - open.height()).abs() < 1.0, "clearing the search grows it back: {open:?} → {again:?}");
}

#[test]
fn the_variable_picker_opened_near_the_bottom_shows_its_whole_list_on_the_screen() {
    let (mut u, r) = scene();
    open(&mut u, r, pos2(900.0, 200.0));
    let full = settled(&mut u);
    let (mut u, r) = scene();
    open(&mut u, r, pos2(900.0, 880.0));
    let low = settled(&mut u);
    assert!((low.height() - full.height()).abs() < 1.0, "as tall as higher up: {full:?} vs {low:?}");
    assert!(low.max.y <= 900.0 && low.min.y >= 0.0, "on the screen: {low:?}");
}
