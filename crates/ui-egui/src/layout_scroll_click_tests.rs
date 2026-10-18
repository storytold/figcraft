//! The layout settings after the panel scrolls: the dots button still toggles the popover at its
//! new place, and clicks inside the popover keep it open and land.

use serde_json::json;

#[test]
fn after_scrolling_the_button_still_toggles_and_the_popover_s_clicks_land() {
    let (mut h, _) = crate::live_tests::Harness::new();
    let frame = figcraft_doc::NodeId(
        h.app.session.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap(),
    );
    h.app.session.execute("layout.add", &json!({"ids": [frame]})).unwrap();
    h.app.session.execute("select.set", &json!({"ids": [frame]})).unwrap();
    h.frames(3);
    let btn = h.app.ui.layout_settings_button.expect("the gap row shows its settings button");
    h.click(btn.center());
    h.frames(2);
    assert!(h.app.ui.layout_settings.is_some(), "the dots opens the settings");
    // Scroll the panel: the button moves; the popover stays where it opened.
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -120.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    };
    h.frame(vec![egui::Event::PointerMoved(egui::pos2(1300.0, 500.0)), wheel]);
    h.frames(8);
    assert!(h.app.ui.layout_settings.is_some(), "scrolling keeps it open");
    // The button's live rect: a click there toggles it shut (no press-close/release-reopen).
    let btn_now = h.app.ui.layout_settings_button.expect("the button is still drawn");
    h.click(btn_now.center());
    h.frames(2);
    assert!(h.app.ui.layout_settings.is_none(), "the button toggles it shut after scrolling");
    // And once more opens it (the button's place is read again: the panel may still be settling).
    let btn_now = h.app.ui.layout_settings_button.expect("the button is still drawn");
    h.click(btn_now.center());
    h.frames(2);
    assert!(h.app.ui.layout_settings.is_some(), "the button opens it again");
    // A click inside the popover keeps it open.
    let pop = h.app.ui.layout_settings.as_ref().expect("open");
    let inside = pop.rect.center();
    h.click(inside);
    h.frames(2);
    assert!(h.app.ui.layout_settings.is_some(), "a click inside keeps it open");
}
