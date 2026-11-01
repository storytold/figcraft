//! Accessibility keys and preferences: F6 / Shift+F6 move the keyboard between the window's regions
//! (the UI does the moving; the commands ask it to), live while editing text too, listed in the
//! shortcuts panel; Reduce motion is a preference.

use serde_json::json;

use crate::Session;
use crate::keymap::{Context, GESTURES, Group, KEYS, Mods, Platform, display, lookup};

#[test]
fn f6_and_shift_f6_cycle_the_regions_on_every_platform_and_while_editing_text() {
    let shift = Mods { shift: true, ..Mods::default() };
    for p in [Platform::Mac, Platform::Other] {
        for c in [Context::Canvas, Context::Text] {
            assert_eq!(lookup(p, c, "F6", Mods::default()).map(|b| b.id), Some("view.nextRegion"), "{p:?} {c:?}");
            assert_eq!(lookup(p, c, "F6", shift).map(|b| b.id), Some("view.previousRegion"), "{p:?} {c:?}");
        }
    }
    assert_eq!(display("F6", Platform::Other), "F6");
    assert_eq!(display("Shift+F6", Platform::Mac), "⇧F6");
    assert_eq!(display("Shift+F6", Platform::Other), "Shift+F6");
    // Listed with the view keys, next to the keyboard-mode keys they lead to.
    for id in ["view.nextRegion", "view.previousRegion"] {
        let b = KEYS.iter().find(|b| b.id == id).unwrap();
        assert_eq!(b.group, Group::View);
        assert!(b.title().starts_with("Focus the"), "{}", b.title());
    }
    assert!(GESTURES.iter().any(|g| g.0 == Group::View && g.3.contains("Tab")));
}

#[test]
fn region_commands_ask_the_ui_to_move_the_keyboard() {
    let mut s = Session::new();
    for (cmd, step) in [("view.nextRegion", 1), ("view.previousRegion", -1)] {
        s.ui_requests.clear();
        s.execute(cmd, &json!({})).unwrap();
        let r = s.ui_requests.iter().find(|r| r.kind == "focus.region").unwrap_or_else(|| panic!("{cmd}"));
        assert_eq!(r.data["step"], step, "{cmd}");
        // Nothing to undo: the document didn't change.
        assert!(!s.ds().unwrap().is_dirty());
    }
}

#[test]
fn reduce_motion_is_a_preference_that_persists() {
    let mut s = Session::new();
    assert!(!s.prefs.reduce_motion);
    assert_eq!(s.execute("pref.reduceMotion", &json!({})).unwrap()["on"], true);
    assert!(s.prefs.reduce_motion);
    assert_eq!(s.execute("pref.reduceMotion", &json!({"on": false})).unwrap()["on"], false);
    s.prefs.reduce_motion = true;
    // Saved with the other preferences; older settings files without it read as off.
    let v = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(v["reduce_motion"], true);
    let old: crate::Prefs = serde_json::from_value(json!({"theme": "dark"})).unwrap();
    assert!(!old.reduce_motion);
}

#[test]
fn interface_size_is_relative_to_the_system_and_persists() {
    let mut s = Session::new();
    assert_eq!(s.prefs.ui_scale, 1.0, "the system's own size by default");
    assert_eq!(s.execute("pref.uiScale", &json!({})).unwrap()["scale"], 1.0);
    assert_eq!(s.execute("pref.uiScale", &json!({"scale": 1.25})).unwrap()["scale"], 1.25);
    assert_eq!(s.prefs.ui_scale, 1.25);
    // Reading changes nothing; "auto" (older scripts) is the system's own size.
    assert_eq!(s.execute("pref.uiScale", &json!({})).unwrap()["scale"], 1.25);
    assert_eq!(s.execute("pref.uiScale", &json!({"scale": "Auto"})).unwrap()["scale"], 1.0);
    // Nothing to undo: the document didn't change.
    assert!(!s.ds().map(|d| d.is_dirty()).unwrap_or(false));
    // Saved with the other preferences and read back.
    s.prefs.ui_scale = 0.9;
    let v = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(v["ui_scale"], 0.9);
    let back: crate::Prefs = serde_json::from_value(v).unwrap();
    assert_eq!(back.ui_scale, 0.9);
}

#[test]
fn interface_size_stops_at_the_ends_and_junk_never_changes_it() {
    let mut s = Session::new();
    assert_eq!(s.execute("pref.uiScale", &json!({"scale": 0.01})).unwrap()["scale"], crate::UI_SCALE_MIN);
    assert_eq!(s.execute("pref.uiScale", &json!({"scale": -4})).unwrap()["scale"], crate::UI_SCALE_MIN);
    assert_eq!(s.execute("pref.uiScale", &json!({"scale": 1e308})).unwrap()["scale"], crate::UI_SCALE_MAX);
    // The size is a factor (1.25), not a percentage: 125 is far past the end and stops there.
    assert_eq!(s.execute("pref.uiScale", &json!({"scale": 125})).unwrap()["scale"], crate::UI_SCALE_MAX);
    s.prefs.ui_scale = 1.5;
    for junk in [json!({"scale": "NaN"}), json!({"scale": "big"}), json!({"scale": [1]}), json!({"scale": {"x": 1}})] {
        assert!(s.execute("pref.uiScale", &junk).is_err(), "{junk}");
        assert_eq!(s.prefs.ui_scale, 1.5, "{junk}");
    }
}

#[test]
fn a_settings_file_without_or_with_a_wrong_interface_size_reads_as_the_systems_size_and_keeps_the_rest() {
    let old: crate::Prefs = serde_json::from_value(json!({"theme": "dark"})).unwrap();
    assert_eq!(old.ui_scale, 1.0);
    for bad in [json!("big"), json!(null), json!([2]), json!(true)] {
        let p: crate::Prefs = serde_json::from_value(json!({"theme": "dark", "ui_scale": bad})).unwrap();
        assert_eq!((p.ui_scale, p.theme.as_str()), (1.0, "dark"), "{bad}");
    }
    // A number out of range is clamped rather than refused.
    let p: crate::Prefs = serde_json::from_value(json!({"ui_scale": 99, "theme": "dark"})).unwrap();
    assert_eq!((p.ui_scale, p.theme.as_str()), (crate::UI_SCALE_MAX, "dark"));
}
