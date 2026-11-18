//! Prototype presentation settings through the session: the device's rotation (saved in the
//! file), the presenter's scale options and explicit variable modes set back to Auto.

use figcraft_doc::NodeKind;
use figcraft_doc::proto::{DeviceRotation, PrototypeSettings};
use serde_json::json;

use crate::Session;
use crate::params::id_of;

fn settings(s: &Session) -> PrototypeSettings {
    let doc = s.doc().unwrap();
    match &doc.node(s.page().unwrap()).unwrap().kind {
        NodeKind::Page { prototype, .. } => prototype.clone(),
        _ => panic!("not a page"),
    }
}

#[test]
fn device_rotation_turns_phones_and_tablets_and_is_saved() {
    let mut s = Session::new();
    s.execute("proto.settings", &json!({"device": "iPhone 16"})).unwrap();
    assert_eq!(settings(&s).rotation, DeviceRotation::Portrait);
    assert_eq!(settings(&s).device().map(|d| (d.width, d.height)), Some((393.0, 852.0)));
    // Landscape swaps the screen's sides.
    s.execute("proto.settings", &json!({"rotation": "landscape"})).unwrap();
    assert!(settings(&s).landscape());
    assert_eq!(settings(&s).device().map(|d| (d.width, d.height)), Some((852.0, 393.0)));
    // Undoable, and degrees work too.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(settings(&s).rotation, DeviceRotation::Portrait);
    s.execute("proto.settings", &json!({"rotation": 90})).unwrap();
    assert!(settings(&s).landscape());
    // Laptops and desktops don't turn: the rotation is kept but doesn't apply.
    s.execute("proto.settings", &json!({"device": "MacBook Air"})).unwrap();
    assert_eq!(settings(&s).rotation, DeviceRotation::Landscape);
    assert!(!settings(&s).landscape());
    assert_eq!(settings(&s).device().map(|d| (d.width, d.height)), Some((1280.0, 832.0)));
    // Junk is refused and changes nothing.
    for bad in [json!("sideways"), json!(45), json!(true), json!({}), json!(f64::MAX)] {
        assert!(s.execute("proto.settings", &json!({"rotation": bad})).is_err(), "{bad}");
    }
    assert_eq!(settings(&s).rotation, DeviceRotation::Landscape);
    // Saved in the file and read back; files without it open in portrait.
    s.execute("proto.settings", &json!({"device": "iPad mini"})).unwrap();
    let bytes = figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    let (doc, _) = figcraft_format::load_bytes(&bytes).unwrap();
    let NodeKind::Page { prototype, .. } = &doc.node(doc.pages[0]).unwrap().kind else { panic!("not a page") };
    assert_eq!(prototype.device().map(|d| (d.width, d.height)), Some((1133.0, 744.0)));
    let old: PrototypeSettings = serde_json::from_value(json!({"device": "iPhone 16"})).unwrap();
    assert_eq!(old.rotation, DeviceRotation::Portrait);
}

#[test]
fn present_options_ask_the_ui_and_check_their_values() {
    let mut s = Session::new();
    s.ui_requests.clear();
    s.execute("proto.presentOptions", &json!({"scale": "fitWidth", "deviceFrame": false})).unwrap();
    let r = s.ui_requests.last().unwrap();
    assert_eq!((r.kind.as_str(), &r.data), ("present.options", &json!({"scale": "fitWidth", "deviceFrame": false})));
    for scale in crate::cmd::proto::PRESENT_SCALES {
        s.execute("proto.presentOptions", &json!({"scale": scale})).unwrap();
    }
    assert!(s.execute("proto.presentOptions", &json!({"scale": "huge"})).is_err());
    assert!(s.execute("proto.presentOptions", &json!({"deviceFrame": "yes"})).is_err());
    assert!(s.execute("proto.presentOptions", &json!({})).is_err(), "nothing to set");
    // It's a viewing option: the document doesn't change.
    assert!(!s.ds().unwrap().is_dirty());
}

#[test]
fn explicit_variable_modes_go_back_to_auto() {
    let mut s = Session::new();
    let id = |v: serde_json::Value| id_of(&v["id"]).unwrap();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10, "parent": f})).unwrap());
    s.execute("variable.createCollection", &json!({"name": "Tokens", "modes": ["Light", "Dark"]})).unwrap();
    s.execute("variable.create", &json!({"collection": "Tokens", "name": "size", "type": "number", "value": 20})).unwrap();
    s.execute("variable.set", &json!({"variable": "size", "mode": "Dark", "value": 40})).unwrap();
    s.execute("variable.bind", &json!({"ids": [r], "field": "width", "variable": "size"})).unwrap();
    let width = |s: &Session| s.doc().unwrap().node(r).unwrap().width;
    s.execute("variable.setMode", &json!({"ids": [f], "collection": "Tokens", "mode": "Dark"})).unwrap();
    assert_eq!(width(&s), 40.0);
    // Auto: the frame inherits the page's mode again, and the layer follows.
    s.execute("variable.setMode", &json!({"ids": [f], "collection": "Tokens", "mode": null})).unwrap();
    assert!(s.doc().unwrap().node(f).unwrap().var_modes.is_empty());
    assert_eq!(width(&s), 20.0);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(width(&s), 40.0);
    // The page set to Dark: Auto inherits it.
    let page = s.page().unwrap();
    s.execute("variable.setMode", &json!({"ids": [page], "collection": "Tokens", "mode": "Dark"})).unwrap();
    s.execute("variable.setMode", &json!({"ids": [f], "collection": "Tokens", "mode": "Light"})).unwrap();
    assert_eq!(width(&s), 20.0);
    s.execute("variable.setMode", &json!({"ids": [f], "collection": "Tokens", "mode": null})).unwrap();
    assert_eq!(width(&s), 40.0);
    assert!(s.execute("variable.setMode", &json!({"ids": [f], "collection": "Tokens", "mode": "Dusk"})).is_err());
    assert!(s.execute("variable.setMode", &json!({"ids": [f], "collection": "Tokens"})).is_err(), "a mode, or null for Auto");
}

/// #2027: an unknown device is refused with the devices to pick from, as an unknown colour is, and
/// nothing changes; a known one is taken whatever its case and stored as the picker names it; the
/// settings read back from `proto.settings` without parameters.
#[test]
fn unknown_devices_are_refused_and_settings_read_back() {
    let mut s = Session::new();
    let before = settings(&s);
    for junk in [json!("Pixel Nonexistent"), json!(""), json!(42), json!(["iPhone 16"])] {
        let err = s.execute("proto.settings", &json!({"device": junk, "responsive": true})).unwrap_err().to_string();
        assert!(err.contains("unknown device") && err.contains("iPhone 17|") && err.contains("Custom size"), "{junk}: {err}");
        assert_eq!(settings(&s), before, "{junk}: nothing changes");
    }
    let set = s.execute("proto.settings", &json!({"device": " iphone 16 ", "color": "black"})).unwrap();
    assert_eq!(settings(&s).device, "iPhone 16", "stored as the picker names it");
    assert_eq!((&set["device"], &set["color"]), (&json!("iPhone 16"), &json!("Black")));
    let read = s.execute("proto.settings", &json!({})).unwrap();
    assert_eq!(read, set, "a call without parameters reads the settings");
    assert_eq!((&read["ok"], &read["rotation"], &read["responsive"]), (&json!(true), &json!("portrait"), &json!(false)));
    assert_eq!(read["screen"], json!({"width": 393.0, "height": 852.0}));
    assert!(read["background"].as_str().is_some_and(|c| c.starts_with('#')), "{read}");
    // "None" and "Custom size" in any case; no device has no screen or colour.
    s.execute("proto.settings", &json!({"device": "custom SIZE", "width": 800, "height": 600})).unwrap();
    assert_eq!(settings(&s).device, PrototypeSettings::CUSTOM_DEVICE);
    let none = s.execute("proto.settings", &json!({"device": "none"})).unwrap();
    assert_eq!((&none["device"], &none["screen"], &none["color"]), (&json!("None"), &json!(null), &json!(null)));
    assert_eq!(none["customSize"], json!({"width": 800.0, "height": 600.0}));
}
