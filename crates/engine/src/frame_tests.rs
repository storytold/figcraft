//! Frame presets: placement beside existing frames, orientation swaps and sizes shared with the
//! prototype devices.

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

/// #1691: the iPhone 17 family is there, and a preset named like a prototype device has its size.
#[test]
fn presets_keep_the_sizes_of_the_prototype_devices() {
    use crate::cmd::create::FRAME_PRESETS;
    use figcraft_doc::proto::PrototypeSettings;
    let mut s = Session::new();
    for (name, size) in
        [("iPhone 17", (402.0, 874.0)), ("iPhone 17 Pro", (402.0, 874.0)), ("iPhone 17 Pro Max", (440.0, 956.0)), ("iPhone Air", (420.0, 912.0))]
    {
        let id = id_of(&s.execute("frame.create", &json!({"preset": name})).unwrap()["id"]).unwrap();
        let n = s.doc().unwrap().node(id).unwrap();
        assert_eq!((n.name.as_str(), n.width, n.height), (name, size.0, size.1));
    }
    let shared = FRAME_PRESETS.iter().filter_map(|p| Some((p, PrototypeSettings::DEVICES.iter().find(|d| d.name == p.1)?)));
    let mut n = 0;
    for ((_, name, w, h), device) in shared {
        assert_eq!((*w, *h), (device.width, device.height), "{name}");
        n += 1;
    }
    assert!(n >= 8, "{n} presets are devices");
}
