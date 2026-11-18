//! The stroke settings' dash pattern: Dash and Gap edit a pair, and the Custom style's Dashes field
//! takes the whole pattern as `dash, gap, dash, gap…` (#1681). A longer pattern shows there, so an
//! imported `[4, 2, 1, 2]` stays whole.

use egui::{Event, Id, Key, Modifiers};
use figcraft_doc::{NodeId, StrokeCap};
use figcraft_engine::Session;
use serde_json::json;

use crate::export_preview_tests::{click, labelled};
use crate::layers_drag_tests::Ui;

/// A selected rectangle with a stroke dashed `dashes`, its advanced stroke settings shown.
fn scene(dashes: serde_json::Value) -> (Ui, NodeId) {
    let mut s = Session::new();
    let r = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 200, "height": 120})).unwrap()["id"].as_u64().unwrap());
    s.execute("node.set", &json!({"ids": [r], "props": {"strokes": ["#000000"], "strokeWeight": 2, "dashes": dashes}})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.data_mut(|d| d.insert_temp(Id::unique("stroke_adv"), true));
    u.frame(vec![]);
    (u, r)
}

fn dashes(u: &Ui, r: NodeId) -> Vec<f64> {
    u.app.session.doc().unwrap().node(r).unwrap().stroke.dashes.clone()
}

/// Whether the field `id` (with `part`) showed in the last frame.
fn shows(u: &Ui, id: &str, part: &str) -> bool {
    u.ctx.read_response(Id::unique(id).with(part)).is_some()
}

/// Click the field `id` (at `part`), type `text` and press Enter.
fn type_in(u: &mut Ui, id: &str, part: &str, text: &str) {
    let at = u.ctx.read_response(Id::unique(id).with(part)).unwrap_or_else(|| panic!("{id} shows")).rect.center();
    u.button(at, true);
    u.button(at, false);
    u.frame(vec![]);
    u.frame(vec![Event::Text(text.into())]);
    u.frame(vec![
        Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
        Event::Key { key: Key::Enter, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
    ]);
    u.frame(vec![]);
}

/// Type `text` into the number field `id` and press Enter.
fn type_into(u: &mut Ui, id: &str, text: &str) {
    type_in(u, id, "field", text);
}

/// Type `text` into the Dashes field and press Enter.
fn type_list(u: &mut Ui, text: &str) {
    type_in(u, "stroke_dashes", "click", text);
}

/// Open the Stroke style menu (showing `shown`) and pick `item`.
fn pick_style(u: &mut Ui, shown: &str, item: &str) {
    let at = |u: &mut Ui, text: &str, last: bool| {
        let all: Vec<_> = labelled(u).into_iter().filter(|(t, _)| t == text).map(|x| x.1).collect();
        // The menu draws over the panel; in the panel the menu's button comes before its fields.
        *(if last { all.last() } else { all.first() }).unwrap_or_else(|| panic!("no {text:?}"))
    };
    let button = at(u, shown, false);
    click(u, button);
    let choice = at(u, item, true);
    click(u, choice);
    u.frame(vec![]);
}

fn toast(u: &Ui) -> String {
    u.app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
}

#[test]
fn a_longer_pattern_shows_as_a_list_and_stays_whole() {
    let (mut u, r) = scene(json!([4, 2, 1, 2]));
    assert!(shows(&u, "stroke_dashes", "click") && !shows(&u, "stroke_dash", "field"), "Custom, not Dash and Gap");
    assert!(labelled(&mut u).iter().any(|(t, _)| t == "4, 2, 1, 2"));
    assert_eq!(dashes(&u, r), [4.0, 2.0, 1.0, 2.0], "showing it changes nothing");
    // The reference's own example: dashes and gaps of any length, as many as you like.
    type_list(&mut u, "10, 20, 10, 20, 80, 20, 10, 100");
    assert_eq!(dashes(&u, r), [10.0, 20.0, 10.0, 20.0, 80.0, 20.0, 10.0, 100.0]);
    // Spaces don't matter, and an odd count is kept as typed (it draws repeated).
    type_list(&mut u, "4,2 ,  1.5,");
    assert_eq!(dashes(&u, r), [4.0, 2.0, 1.5]);
    assert!(labelled(&mut u).iter().any(|(t, _)| t == "4, 2, 1.5"));
}

#[test]
fn only_numbers_go_in_the_list() {
    let (mut u, r) = scene(json!([4, 2, 1, 2]));
    for bad in ["4, x", "-1, 2", "inf", ",", " "] {
        u.app.ui.toast = None;
        type_list(&mut u, bad);
        assert_eq!(dashes(&u, r), [4.0, 2.0, 1.0, 2.0], "{bad:?} changes nothing");
        assert_eq!(toast(&u), "Dashes take numbers separated by commas", "{bad:?} says why");
    }
    // Leaving the text as it was isn't an edit.
    u.app.ui.toast = None;
    type_list(&mut u, "4, 2, 1, 2");
    assert_eq!(toast(&u), "");
}

/// Custom on a plain pair keeps the pattern and shows it as a list, and stays that way after a
/// list of one pair is typed; Dash goes back to the fields with the first pair of the list.
#[test]
fn the_style_menu_moves_between_the_fields_and_the_list() {
    let (mut u, r) = scene(json!([6, 3]));
    assert!(shows(&u, "stroke_dash", "field") && !shows(&u, "stroke_dashes", "click"));
    pick_style(&mut u, "Dash", "Custom");
    assert!(shows(&u, "stroke_dashes", "click") && !shows(&u, "stroke_dash", "field"));
    assert_eq!(dashes(&u, r), [6.0, 3.0], "choosing it changes nothing");
    type_list(&mut u, "5, 5");
    assert_eq!(dashes(&u, r), [5.0, 5.0]);
    assert!(shows(&u, "stroke_dashes", "click"), "still the list");
    type_list(&mut u, "8, 2, 1, 2");
    pick_style(&mut u, "Custom", "Dash");
    assert_eq!(dashes(&u, r), [8.0, 2.0], "the first dash and gap");
    assert!(shows(&u, "stroke_dash", "field") && !shows(&u, "stroke_dashes", "click"));
    pick_style(&mut u, "Dash", "Solid");
    assert_eq!(dashes(&u, r), Vec::<f64>::new());
    // From Solid, a list starts from the same pair Dash does: four stroke widths.
    pick_style(&mut u, "Solid", "Custom");
    assert_eq!(dashes(&u, r), [8.0, 8.0]);
    assert!(shows(&u, "stroke_dashes", "click"));
}

/// Custom chosen for one layer doesn't follow the selection to another.
#[test]
fn a_chosen_list_belongs_to_its_selection() {
    let (mut u, r) = scene(json!([6, 3]));
    pick_style(&mut u, "Dash", "Custom");
    let other = NodeId(u.app.run("shape.rectangle", json!({"x": 300, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap());
    u.app.run("node.set", json!({"ids": [other], "props": {"strokes": ["#000000"], "dashes": [6, 3]}}));
    u.app.run("select.set", json!({"ids": [other]}));
    u.frame(vec![]);
    u.frame(vec![]);
    assert!(shows(&u, "stroke_dash", "field"), "a pair shows as Dash and Gap");
    u.app.run("select.set", json!({"ids": [r]}));
    u.frame(vec![]);
    u.frame(vec![]);
    assert!(shows(&u, "stroke_dashes", "click"), "back on the first layer, the list");
}

#[test]
fn a_single_value_pattern_is_the_dash_and_the_gap() {
    let (mut u, r) = scene(json!([5]));
    type_into(&mut u, "stroke_gap", "2");
    assert_eq!(dashes(&u, r), [5.0, 2.0]);
    type_into(&mut u, "stroke_dash", "7");
    assert_eq!(dashes(&u, r), [7.0, 2.0]);
}

/// Both dashed styles offer the Dash cap, and picking one sets it. Solid has no dashes to cap.
#[test]
fn the_dash_cap_shows_for_dash_and_custom_strokes() {
    let has = |u: &mut Ui, text: &str| labelled(u).iter().any(|(t, _)| t == text);
    let cap = |u: &Ui, r| u.app.session.doc().unwrap().node(r).unwrap().stroke.dash_cap;
    let (mut u, _) = scene(json!([]));
    assert!(!has(&mut u, "Dash cap"));
    let (mut u, r) = scene(json!([6, 3]));
    assert!(has(&mut u, "Dash cap"));
    assert_eq!(cap(&u, r), None, "showing it changes nothing");
    pick_style(&mut u, "None", "Round");
    assert_eq!(cap(&u, r), Some(StrokeCap::Round));
    pick_style(&mut u, "Dash", "Custom");
    assert!(has(&mut u, "Dash cap"));
    pick_style(&mut u, "Round", "Square");
    assert_eq!(cap(&u, r), Some(StrokeCap::Square));
    assert_eq!(dashes(&u, r), [6.0, 3.0]);
}

/// Unset, the setting shows the end point caps the dashes take.
#[test]
fn an_unset_dash_cap_shows_what_draws() {
    let (mut u, r) = scene(json!([6, 3]));
    let caps = |u: &mut Ui, start: &str, end: &str| {
        u.app.run("node.set", json!({"ids": [r], "props": {"startCap": start, "endCap": end}}));
        u.frame(vec![]);
        u.frame(vec![]);
    };
    caps(&mut u, "round", "round");
    assert!(labelled(&mut u).iter().any(|(t, _)| t == "Round"));
    caps(&mut u, "round", "square");
    assert!(labelled(&mut u).iter().any(|(t, _)| t == "Mixed"));
    assert_eq!(u.app.session.doc().unwrap().node(r).unwrap().stroke.dash_cap, None);
}
