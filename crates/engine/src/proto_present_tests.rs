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
fn prototype_background_uses_the_theme_until_a_colour_is_chosen() {
    let mut s = Session::new();
    assert!(settings(&s).uses_theme_background());
    // Choosing even the old default explicitly must keep it through a file round trip.
    s.execute("proto.settings", &json!({"background": "#1E1E1E"})).unwrap();
    assert!(!settings(&s).uses_theme_background());
    let bytes = figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    let (doc, _) = figcraft_format::load_bytes(&bytes).unwrap();
    let NodeKind::Page { prototype, .. } = &doc.node(doc.pages[0]).unwrap().kind else { panic!("not a page") };
    assert!(!prototype.uses_theme_background());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(settings(&s).uses_theme_background());
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
