//! The stroke settings' Dash and Gap fields edit the first pair of a dash pattern and keep the
//! rest of it (#1681): an imported `[4, 2, 1, 2]` used to collapse to `[dash, gap]` on the first edit.

use egui::{Event, Id, Key, Modifiers};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

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

/// Type `text` into the number field `id` and press Enter.
fn type_into(u: &mut Ui, id: &str, text: &str) {
    let at = u.ctx.read_response(Id::unique(id).with("field")).unwrap_or_else(|| panic!("{id} shows")).rect.center();
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

#[test]
fn editing_the_dash_or_gap_keeps_the_rest_of_the_pattern() {
    let (mut u, r) = scene(json!([4, 2, 1, 2]));
    type_into(&mut u, "stroke_dash", "6");
    assert_eq!(dashes(&u, r), [6.0, 2.0, 1.0, 2.0]);
    type_into(&mut u, "stroke_gap", "3");
    assert_eq!(dashes(&u, r), [6.0, 3.0, 1.0, 2.0]);
}

#[test]
fn a_single_value_pattern_is_the_dash_and_the_gap() {
    let (mut u, r) = scene(json!([5]));
    type_into(&mut u, "stroke_gap", "2");
    assert_eq!(dashes(&u, r), [5.0, 2.0]);
}

/// An odd-length pattern draws twice over (`[4, 2, 1]` is `[4, 2, 1, 4, 2, 1]`): editing its Dash
/// writes the repeat out, so only the first dash changes and every gap stays as drawn.
#[test]
fn an_odd_length_pattern_keeps_its_repeat() {
    let (mut u, r) = scene(json!([4, 2, 1]));
    type_into(&mut u, "stroke_dash", "6");
    assert_eq!(dashes(&u, r), [6.0, 2.0, 1.0, 4.0, 2.0, 1.0]);
    type_into(&mut u, "stroke_gap", "3");
    assert_eq!(dashes(&u, r), [6.0, 3.0, 1.0, 4.0, 2.0, 1.0]);
}
