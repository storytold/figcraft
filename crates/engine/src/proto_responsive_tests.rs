//! Responsive presentation (frames laid out again for a screen: constraints and auto layout
//! respond), device colours and custom device sizes: the settings through `proto.settings`, saved
//! in the file, and [`respond`] on a document.

use figcraft_doc::proto::PrototypeSettings;
use figcraft_doc::{Document, NodeId, NodeKind};
use serde_json::{Value, json};

use crate::Session;
use crate::cmd::proto::respond;
use crate::params::id_of;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    match s.execute(cmd, &p) {
        Ok(v) => v,
        Err(e) => panic!("{cmd} {p}: {e}"),
    }
}

fn idv(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn settings(s: &Session) -> PrototypeSettings {
    let doc = s.doc().unwrap();
    match &doc.node(s.page().unwrap()).unwrap().kind {
        NodeKind::Page { prototype, .. } => prototype.clone(),
        _ => panic!("not a page"),
    }
}

fn set(s: &mut Session, id: NodeId, props: Value) {
    ex(s, "node.set", json!({"id": id, "props": props}));
}

/// `id`'s box in its parent: (x, y, width, height).
fn rect(doc: &Document, id: NodeId) -> (f64, f64, f64, f64) {
    let n = doc.node(id).unwrap();
    let c = n.transform.as_coeffs();
    (c[4], c[5], n.width, n.height)
}

/// A device name is matched as a colour's is, in any case, and stored as the list writes it; a
/// name that is no device is refused with the list and changes nothing, nor records a step
/// (#2027).
#[test]
fn an_unknown_device_is_refused_with_the_devices() {
    let mut s = Session::new();
    ex(&mut s, "proto.settings", json!({"device": "iPhone 16"}));
    let steps = s.ds().unwrap().history.undo.len();
    let before = settings(&s);
    for bad in [json!("Pixel Nonexistent"), json!("iPhone 16 Mini"), json!(""), json!(16), json!(true), json!({}), json!(["iPhone 16"])] {
        let e = s.execute("proto.settings", &json!({"device": bad, "color": "Black"})).unwrap_err().to_string();
        assert!(e.contains("iPhone 16 Pro") && e.contains("Custom size") && e.contains("None"), "{bad}: {e}");
        assert_eq!(settings(&s), before, "{bad}: nothing changes");
    }
    assert_eq!(s.ds().unwrap().history.undo.len(), steps, "no step for a refusal");
    for (given, stored) in
        [("iphone 16 pro", "iPhone 16 Pro"), ("  iPad Mini ", "iPad mini"), ("custom size", "Custom size"), ("none", "None"), ("DESKTOP", "Desktop")]
    {
        ex(&mut s, "proto.settings", json!({"device": given}));
        assert_eq!(settings(&s).device, stored, "{given}");
        assert!(stored == "None" || settings(&s).model().is_some(), "{given}: the presenter finds the model");
    }
}

#[test]
fn device_colours_pick_by_name_and_follow_the_device() {
    let mut s = Session::new();
    // Every device model comes in colours (the first is the default); names are unique per model.
    for d in PrototypeSettings::DEVICES {
        assert!(!d.colors.is_empty(), "{}", d.name);
        let mut names: Vec<&str> = d.colors.iter().map(|c| c.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), d.colors.len(), "{}", d.name);
    }
    ex(&mut s, "proto.settings", json!({"device": "iPhone 16"}));
    assert_eq!(settings(&s).device_color().map(|c| c.name), Some("Black"), "the first by default");
    // Case doesn't matter; the stored name is the model's.
    ex(&mut s, "proto.settings", json!({"color": "teal"}));
    assert_eq!(settings(&s).color, "Teal");
    assert_eq!(settings(&s).device_color().map(|c| c.rgb), Some([0xA3, 0xCF, 0xC6]));
    // A colour the device doesn't come in is refused and changes nothing.
    for bad in [json!("Plaid"), json!(3), json!({}), json!(["Teal"])] {
        assert!(s.execute("proto.settings", &json!({"color": bad})).is_err(), "{bad}");
    }
    assert_eq!(settings(&s).color, "Teal");
    // Another device keeps the colour only if it comes in it; an empty name is the default.
    ex(&mut s, "proto.settings", json!({"device": "iPad mini", "color": "Blue"}));
    assert_eq!(settings(&s).device_color().map(|c| c.name), Some("Blue"));
    ex(&mut s, "proto.settings", json!({"device": "MacBook Air"}));
    assert_eq!((settings(&s).color.as_str(), settings(&s).device_color().map(|c| c.name)), ("", Some("Silver")));
    ex(&mut s, "proto.settings", json!({"color": "Grey"}));
    ex(&mut s, "proto.settings", json!({"color": ""}));
    assert_eq!(settings(&s).device_color().map(|c| c.name), Some("Silver"));
    // Undoable; no device, no colours.
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(settings(&s).color, "Grey");
    ex(&mut s, "proto.settings", json!({"device": "None"}));
    assert_eq!(settings(&s).device_color(), None);
    assert!(s.execute("proto.settings", &json!({"color": "Black"})).is_err());
}

#[test]
fn custom_sizes_are_a_screen_of_their_own() {
    let mut s = Session::new();
    // Width or height alone picks "Custom size", keeping the other side.
    ex(&mut s, "proto.settings", json!({"width": 1024}));
    let p = settings(&s);
    assert_eq!(p.device, PrototypeSettings::CUSTOM_DEVICE);
    let d = p.device().unwrap();
    assert_eq!((d.width, d.height, d.rotates), (1024.0, 720.0, false));
    assert!(d.colors.is_empty(), "just a screen");
    assert_eq!(p.device_color(), None);
    ex(&mut s, "proto.settings", json!({"width": 360.4, "height": 640}));
    assert_eq!(settings(&s).device().map(|d| (d.width, d.height)), Some((360.0, 640.0)));
    // It doesn't turn: the sides are the screen.
    ex(&mut s, "proto.settings", json!({"rotation": "landscape"}));
    assert_eq!(settings(&s).device().map(|d| (d.width, d.height)), Some((360.0, 640.0)));
    // Out of range or not a number: refused, nothing changes (huge sizes are capped).
    for bad in [json!(0), json!(-5), json!("wide"), json!(0.5), json!(true)] {
        assert!(s.execute("proto.settings", &json!({"width": bad})).is_err(), "{bad}");
        assert!(s.execute("proto.settings", &json!({"height": bad, "device": "iPhone 16"})).is_err(), "{bad}");
    }
    assert_eq!(settings(&s).device().map(|d| (d.width, d.height)), Some((360.0, 640.0)));
    ex(&mut s, "proto.settings", json!({"height": 1e12}));
    assert_eq!(settings(&s).device().map(|d| d.height), Some(PrototypeSettings::CUSTOM_MAX));
    // Picking another device keeps the custom size for later.
    ex(&mut s, "proto.settings", json!({"device": "Desktop"}));
    ex(&mut s, "proto.settings", json!({"device": "Custom size"}));
    assert_eq!(settings(&s).device().map(|d| d.width), Some(360.0));
    // A file with junk in it still presents something sensible.
    let junk: PrototypeSettings = serde_json::from_value(json!({"device": "Custom size", "custom_size": [-1, 1e300]})).unwrap();
    assert_eq!(junk.device().map(|d| (d.width, d.height)), Some((1280.0, PrototypeSettings::CUSTOM_MAX)));
}

#[test]
fn settings_are_saved_and_old_files_read_as_before() {
    let mut s = Session::new();
    ex(&mut s, "proto.settings", json!({"device": "Android Compact", "color": "Green", "responsive": true}));
    ex(&mut s, "proto.settings", json!({"width": 800, "height": 600}));
    ex(&mut s, "proto.settings", json!({"device": "Android Compact", "color": "Green"}));
    let bytes = figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    let (doc, _) = figcraft_format::load_bytes(&bytes).unwrap();
    let NodeKind::Page { prototype, .. } = &doc.node(doc.pages[0]).unwrap().kind else { panic!("not a page") };
    assert_eq!(prototype, &settings(&s));
    assert!(prototype.responsive);
    assert_eq!((prototype.color.as_str(), prototype.custom_size), ("Green", [800.0, 600.0]));
    // Defaults aren't written; files without the fields read as before.
    let plain = serde_json::to_value(PrototypeSettings::default()).unwrap();
    for k in ["color", "custom_size", "responsive"] {
        assert!(plain.get(k).is_none(), "{plain}");
    }
    let old: PrototypeSettings = serde_json::from_value(json!({"device": "iPhone 16", "rotation": "landscape"})).unwrap();
    assert!(!old.responsive);
    assert_eq!(old.device_color().map(|c| c.name), Some("Black"));
    assert_eq!(old.custom_size, PrototypeSettings::CUSTOM_DEFAULT);
    // Responsive must be a boolean.
    assert!(s.execute("proto.settings", &json!({"responsive": "yes"})).is_err());
    ex(&mut s, "proto.settings", json!({"responsive": false}));
    assert!(!settings(&s).responsive);
}

#[test]
fn responding_frames_follow_constraints_and_auto_layout() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 50, "y": 20, "width": 400, "height": 300})));
    let right = idv(&ex(&mut s, "shape.rectangle", json!({"x": 340, "y": 10, "width": 50, "height": 20, "parent": f})));
    set(&mut s, right, json!({"constraints": {"horizontal": "right", "vertical": "top"}}));
    let stretch = idv(&ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 250, "width": 380, "height": 40, "parent": f})));
    set(&mut s, stretch, json!({"constraints": {"horizontal": "leftRight", "vertical": "bottom"}}));
    let centred = idv(&ex(&mut s, "shape.rectangle", json!({"x": 175, "y": 125, "width": 50, "height": 50, "parent": f})));
    set(&mut s, centred, json!({"constraints": {"horizontal": "center", "vertical": "center"}}));
    // An auto layout row inside, stretching with the frame, whose fill child takes the free width.
    let row = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 60, "width": 400, "height": 40, "parent": f})));
    ex(&mut s, "layout.add", json!({"ids": [row], "direction": "horizontal"}));
    set(&mut s, row, json!({"sizingH": "fixed", "width": 400, "sizingV": "fixed", "height": 40, "gap": 0, "padding": 0}));
    set(&mut s, row, json!({"constraints": {"horizontal": "leftRight", "vertical": "top"}}));
    let fixed = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 40, "parent": row})));
    let fill = idv(&ex(&mut s, "shape.rectangle", json!({"x": 100, "y": 0, "width": 300, "height": 40, "parent": row})));
    set(&mut s, fill, json!({"sizingH": "fill"}));
    let before = s.doc().unwrap().clone();
    assert_eq!(rect(&before, fill).2, 300.0);

    let mut d = before.clone();
    respond(&mut d, f, (600.0, 500.0));
    assert_eq!(rect(&d, f), (50.0, 20.0, 600.0, 500.0), "keeps its place");
    assert_eq!(rect(&d, right), (540.0, 10.0, 50.0, 20.0));
    assert_eq!(rect(&d, stretch), (10.0, 450.0, 580.0, 40.0));
    assert_eq!(rect(&d, centred), (275.0, 225.0, 50.0, 50.0));
    assert_eq!(rect(&d, row).2, 600.0);
    assert_eq!(rect(&d, fixed).2, 100.0);
    assert_eq!(rect(&d, fill), (100.0, 0.0, 500.0, 40.0), "the fill child takes the new room");
    // From the original again (never from a previous size): smaller this time.
    let mut d = before.clone();
    respond(&mut d, f, (300.0, 200.0));
    assert_eq!(rect(&d, right).0, 240.0);
    assert_eq!(rect(&d, fill).2, 200.0);
    // The editor's document is untouched; junk sizes and ids change nothing or stay finite.
    assert_eq!(s.doc().unwrap().node(f).unwrap().width, 400.0);
    for size in [(f64::NAN, 10.0), (10.0, f64::INFINITY)] {
        let mut d = before.clone();
        respond(&mut d, f, size);
        assert_eq!(rect(&d, f), rect(&before, f));
    }
    let mut d = before.clone();
    respond(&mut d, f, (0.0, 1e12));
    assert_eq!((rect(&d, f).2, rect(&d, f).3), (1.0, 100_000.0));
    assert!(d.subtree(f).iter().all(|i| {
        let r = rect(&d, *i);
        r.0.is_finite() && r.1.is_finite() && r.2 >= 0.0 && r.3 >= 0.0
    }));
    respond(&mut d, right, (10.0, 10.0));
    respond(&mut d, NodeId(987_654), (10.0, 10.0));
    assert!(d.validate().is_empty(), "{:?}", d.validate());
}

#[test]
fn a_frame_that_hugs_keeps_hugging_its_content() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 300})));
    ex(&mut s, "layout.add", json!({"ids": [f], "direction": "vertical"}));
    set(&mut s, f, json!({"sizingH": "fixed", "width": 400, "sizingV": "hug", "gap": 0, "padding": 0}));
    let item = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 400, "height": 900, "parent": f})));
    set(&mut s, item, json!({"sizingH": "fill"}));
    let before = s.doc().unwrap().clone();
    assert_eq!(rect(&before, f).3, 900.0);
    let mut d = before.clone();
    respond(&mut d, f, (393.0, 852.0));
    // The width is the screen's; the height still hugs the content (it scrolls on the screen).
    assert_eq!((rect(&d, f).2, rect(&d, f).3), (393.0, 900.0));
    assert_eq!(rect(&d, item).2, 393.0);
}
