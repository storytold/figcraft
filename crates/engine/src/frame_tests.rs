//! Frame presets: placement beside existing frames and orientation swaps.

use serde_json::json;

use crate::Session;
use crate::params::id_of;

#[test]
fn presets_land_beside_existing_frames_and_swap_orientation() {
    let mut s = Session::new();
    let a = id_of(&s.execute("frame.create", &json!({"preset": "iPhone 16"})).unwrap()["id"]).unwrap();
    let ba = s.doc().unwrap().world_bounds(a).unwrap();
    assert_eq!((ba.width(), ba.height()), (393.0, 852.0));
    let view = s.ds().unwrap().view.clone();
    assert!((ba.center().x - view.cx).abs() <= 1.0 && (ba.center().y - view.cy).abs() <= 1.0, "first one in the view");
    let b = id_of(&s.execute("frame.create", &json!({"preset": "desktop"})).unwrap()["id"]).unwrap();
    let bb = s.doc().unwrap().world_bounds(b).unwrap();
    assert_eq!((bb.x0, bb.y0), (ba.x1 + 100.0, ba.y0), "to the right, top aligned");
    assert_eq!(s.doc().unwrap().node(b).unwrap().name, "Desktop");
    // Explicit positions still win.
    let c = id_of(&s.execute("frame.create", &json!({"preset": "A4", "x": 5, "y": 6})).unwrap()["id"]).unwrap();
    assert_eq!(s.doc().unwrap().world_bounds(c).unwrap().x0, 5.0);
    assert!(s.execute("frame.create", &json!({"preset": "Nope"})).is_err());
    // Swap orientation, one undo step.
    s.execute("frame.swapOrientation", &json!({"ids": [a]})).unwrap();
    let n = s.doc().unwrap().node(a).unwrap().clone();
    assert_eq!((n.width, n.height), (852.0, 393.0));
    s.undo().unwrap();
    assert_eq!(s.doc().unwrap().node(a).unwrap().width, 393.0);
}

#[test]
fn custom_presets_persist_update_create_and_delete() {
    let mut s = Session::new();
    s.execute("frame.presetSave", &json!({"name": " Banner ", "width": 640, "height": 120})).unwrap();
    s.execute("frame.presetSave", &json!({"name": "banner", "width": 720.5, "height": 160})).unwrap();
    assert_eq!(s.prefs.frame_presets.len(), 1);
    let saved = serde_json::to_value(&s.prefs).unwrap();
    let mut restored = Session::new();
    restored.prefs = serde_json::from_value(saved).unwrap();
    let list = restored.execute("frame.presets", &json!({})).unwrap();
    assert!(list.as_array().unwrap().iter().any(|p| p["category"] == "Custom" && p["name"] == "banner" && p["width"] == 720.5));
    let built_in = id_of(&restored.execute("frame.create", &json!({"preset": "A4"})).unwrap()["id"]).unwrap();
    let bounds = restored.doc().unwrap().world_bounds(built_in).unwrap();
    let id = id_of(&restored.execute("frame.create", &json!({"preset": "BANNER"})).unwrap()["id"]).unwrap();
    let n = restored.doc().unwrap().node(id).unwrap();
    assert_eq!((n.width, n.height), (720.5, 160.0));
    assert_eq!(n.name, "banner");
    assert_eq!(restored.doc().unwrap().world_bounds(id).unwrap().x0, bounds.x1 + 100.0);
    assert_eq!(restored.selection(), vec![id]);
    restored.undo().unwrap();
    assert!(!restored.doc().unwrap().contains(id));
    assert_eq!(restored.prefs.frame_presets.len(), 1);
    restored.redo().unwrap();
    assert!(restored.doc().unwrap().contains(id));
    restored.execute("frame.presetDelete", &json!({"name": "BANNER"})).unwrap();
    assert!(restored.prefs.frame_presets.is_empty());
    assert!(restored.doc().unwrap().contains(id), "deleting a preset keeps its frames");
    assert!(restored.execute("frame.create", &json!({"preset": "banner"})).is_err());
    let old_prefs: crate::Prefs = serde_json::from_value(json!({"theme": "dark"})).unwrap();
    assert!(old_prefs.frame_presets.is_empty(), "older preferences remain readable");
}

#[test]
fn custom_presets_reject_invalid_values_and_bound_storage() {
    let mut s = Session::new();
    for invalid in [
        json!({"name": "", "width": 10, "height": 10}),
        json!({"name": "iPHONE 16", "width": 10, "height": 10}),
        json!({"name": "a".repeat(65), "width": 10, "height": 10}),
        json!({"name": "a\nb", "width": 10, "height": 10}),
        json!({"name": "test", "width": 0, "height": 10}),
        json!({"name": "test", "width": 10, "height": -1}),
        json!({"name": "test", "width": "NaN", "height": 10}),
        json!({"name": "test", "width": 10, "height": 1000001}),
        json!({"name": "test", "width": 10}),
    ] {
        assert!(s.execute("frame.presetSave", &invalid).is_err(), "accepted {invalid}");
        assert!(s.prefs.frame_presets.is_empty());
    }
    for i in 0..100 {
        s.execute("frame.presetSave", &json!({"name": format!("size {i}"), "width": 10, "height": 20})).unwrap();
    }
    assert!(s.execute("frame.presetSave", &json!({"name": "overflow", "width": 10, "height": 20})).is_err());
    s.execute("frame.presetSave", &json!({"name": "SIZE 0", "width": 30, "height": 40})).unwrap();
    assert_eq!(s.prefs.frame_presets.len(), 100);
    assert!(s.execute("frame.presetDelete", &json!({"name": "A4"})).is_err());
    assert!(s.execute("frame.presetDelete", &json!({"name": "missing"})).is_err());
}
