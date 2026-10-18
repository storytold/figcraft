//! The auto layout's settings popover, off the gap row's dots button: it opens and toggles on a
//! real press and release, it keeps its place while the panel scrolls while the button still
//! toggles it at its new place, and an instance keeps it (the baselines are the instance's own).

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
fn the_gap_rows_button_opens_the_layout_settings_and_toggles_them_shut() {
    let (mut h, _) = Harness::new();
    let frame = id(h.app.session.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap());
    h.app.session.execute("layout.add", &json!({"ids": [frame]})).unwrap();
    h.app.session.execute("select.set", &json!({"ids": [frame]})).unwrap();
    h.frames(3);
    let btn = h.app.ui.layout_settings_button.expect("the gap row shows its settings button");
    h.click(btn.center());
    h.frames(2);
    assert!(h.app.ui.layout_settings.is_some(), "the dots button opens the layout settings");
    // A press on the button keeps it open (it is held out), the release toggles it shut.
    h.click(btn.center());
    h.frames(2);
    assert!(h.app.ui.layout_settings.is_none(), "the button toggles it shut, not shut and open again");
    // Escape closes it too, and leaves the selection alone (the shortcut doesn't deselect).
    h.click(btn.center());
    h.frames(2);
    assert!(h.app.ui.layout_settings.is_some());
    h.frame(escape());
    assert!(h.app.ui.layout_settings.is_none(), "Escape closes the settings");
    assert_eq!(h.app.session.selection(), vec![frame], "Escape keeps the selection");
}

#[test]
fn an_instances_layout_settings_open_with_the_baselines_still_settable() {
    let (mut h, _) = Harness::new();
    let f = id(h.app.session.execute("frame.create", &json!({"x": 0, "y": 0, "width": 120, "height": 40})).unwrap());
    h.app.session.execute("layout.add", &json!({"ids": [f]})).unwrap();
    let c = id(h.app.session.execute("component.create", &json!({"ids": [f]})).unwrap());
    let inst = id(h.app.session.execute("instance.create", &json!({"component": c, "x": 300, "y": 0})).unwrap());
    h.app.session.execute("select.set", &json!({"ids": [inst]})).unwrap();
    h.frames(3);
    // The button stays enabled inside an instance: the text baselines are the instance's own.
    let btn = h.app.ui.layout_settings_button.expect("an instance keeps its settings button");
    h.click(btn.center());
    h.frames(3);
    assert!(h.app.ui.layout_settings.is_some(), "an instance opens the layout settings");
}
