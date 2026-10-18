//! The stroke's advanced settings popover, off the stroke row's dots button: it opens and toggles
//! on a real press and release, it stays open for a selection whose stroke settings differ, and
//! Escape in one of its fields ends that edit before it closes the popover.

use egui::{Event, Key};
use serde_json::json;

use crate::live_tests::Harness;

fn id(v: serde_json::Value) -> figcraft_doc::NodeId {
    figcraft_doc::NodeId(v["id"].as_u64().unwrap_or_default())
}

fn escape() -> Vec<Event> {
    vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]
}

#[test]
fn the_stroke_rows_dots_button_opens_the_advanced_stroke_settings() {
    let (mut h, _) = Harness::new();
    let rect = id(h.app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    h.app.session.execute("stroke.add", &json!({"ids": [rect]})).unwrap();
    h.app.session.execute("select.set", &json!({"ids": [rect]})).unwrap();
    h.frames(3);
    let btn = h.app.ui.stroke_settings_button.expect("the stroke row shows its advanced settings button");
    h.click(btn.center());
    h.frames(2);
    assert!(h.app.ui.stroke_settings.is_some(), "the dots button opens the advanced stroke settings");
    h.click(btn.center());
    h.frames(2);
    assert!(h.app.ui.stroke_settings.is_none(), "the button toggles them shut");
}

#[test]
fn the_stroke_settings_open_from_the_button_when_the_layers_settings_differ() {
    let (mut h, _) = Harness::new();
    let a = id(h.app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap());
    let b = id(h.app.session.execute("shape.rectangle", &json!({"x": 100, "y": 0, "width": 40, "height": 40})).unwrap());
    // The same stroke on both (one row, so the dots button is there), different settings.
    h.app.session.execute("stroke.add", &json!({"ids": [a, b]})).unwrap();
    h.app.session.execute("node.set", &json!({"ids": [a], "props": {"strokeWeight": 3}})).unwrap();
    h.app.session.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    h.frames(3);
    let btn = h.app.ui.stroke_settings_button.expect("the stroke row shows its advanced settings button");
    h.click(btn.center());
    h.frames(3);
    assert!(h.app.ui.stroke_settings.is_some(), "mixed stroke settings keep the popover open");
}

#[test]
fn escape_in_a_popover_field_ends_the_edit_and_leaves_the_popover_open() {
    let (mut h, _) = Harness::new();
    let rect = id(h.app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    h.app.session.execute("stroke.add", &json!({"ids": [rect]})).unwrap();
    // Dashed, so the popover has its dash and gap number fields.
    h.app.session.execute("node.set", &json!({"ids": [rect], "props": {"dashes": [4.0, 4.0]}})).unwrap();
    h.app.session.execute("select.set", &json!({"ids": [rect]})).unwrap();
    h.frames(3);
    let btn = h.app.ui.stroke_settings_button.expect("the stroke row shows its advanced settings button");
    h.click(btn.center());
    h.frames(3);
    let dash = h.ctx.read_response(egui::Id::new("stroke_dash").with("field")).expect("the dash field is drawn");
    h.click(dash.rect.center());
    h.frames(2);
    // The first Escape ends the field's edit only.
    h.frame(escape());
    h.frames(1);
    assert!(h.app.ui.stroke_settings.is_some(), "Escape in a field leaves the popover open");
    // The second has nothing left to close but the popover.
    h.frame(escape());
    assert!(h.app.ui.stroke_settings.is_none(), "Escape then closes the popover");
}
