//! Responsive presentation in the player (the frame takes the device's screen or the window and
//! lays out again, live), device colours and custom sizes in the presenter and the Prototype
//! panel, and library collections in the Design panel's variable modes.

use egui::{Key, pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::present_tests::{Ui, id, setup};

fn presenter(u: &Ui) -> Value {
    u.app.inspect()["presenter"].clone()
}

fn size(v: &Value) -> (f64, f64) {
    (v[0].as_f64().unwrap(), v[1].as_f64().unwrap())
}

/// A 600×400 frame "Wide" with a button pinned to its right edge, and a 300×200 frame "Small".
fn wide(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let ex = |s: &mut Session, c: &str, p: Value| s.execute(c, &p).unwrap();
    let w = id(&ex(s, "node.create", json!({"type": "frame", "name": "Wide", "x": 0, "y": 0, "width": 600, "height": 400})));
    let button = id(&ex(s, "node.create", json!({"type": "rectangle", "parent": w, "x": 500, "y": 10, "width": 90, "height": 30})));
    ex(s, "node.set", json!({"id": button, "props": {"constraints": {"horizontal": "right", "vertical": "top"}}}));
    let small = id(&ex(s, "node.create", json!({"type": "frame", "name": "Small", "x": 700, "y": 0, "width": 300, "height": 200})));
    (w, button, small)
}

#[test]
fn responsive_frames_take_the_devices_screen_live() {
    let mut s = Session::new();
    s.execute("proto.settings", &json!({"device": "iPhone 16", "color": "Pink"})).unwrap();
    let (w, button, small) = wide(&mut s);
    let mut u = Ui::new(s);
    u.run("proto.present", json!({"start": w}));
    let p = presenter(&u);
    assert_eq!((p["responsive"].clone(), p["deviceColor"].clone()), (Value::Null, json!("Pink")));
    assert_eq!(size(&p["frameSize"]), (600.0, 400.0), "fixed size: the frame as designed");
    // Turned on while presenting: the frame takes the screen, and its layers respond.
    u.run("proto.settings", json!({"responsive": true}));
    let p = presenter(&u);
    assert_eq!(size(&p["responsive"]), (393.0, 852.0));
    assert_eq!(size(&p["frameSize"]), (393.0, 852.0));
    assert_eq!(size(&p["viewport"]), (393.0, 852.0));
    assert_eq!(u.app.session.doc().unwrap().node(w).unwrap().width, 600.0, "the file keeps its frame");
    // Turned to landscape: the frame follows the screen.
    u.run("proto.settings", json!({"rotation": "landscape"}));
    assert_eq!(size(&presenter(&u)["frameSize"]), (852.0, 393.0));
    // Edits show live and are laid out again; the button stays pinned to the right.
    u.run("node.set", json!({"id": button, "props": {"width": 40}}));
    let laid = presenter(&u);
    assert_eq!(size(&laid["frameSize"]), (852.0, 393.0));
    // The next frame responds too.
    u.key(pos2(720.0, 470.0), Key::ArrowRight);
    let p = presenter(&u);
    assert_eq!(p["current"], json!(small));
    assert_eq!(size(&p["frameSize"]), (852.0, 393.0));
    // Off again: back to the frames as designed.
    u.run("proto.settings", json!({"responsive": false}));
    assert_eq!(size(&presenter(&u)["frameSize"]), (300.0, 200.0));
}

#[test]
fn without_a_device_a_responsive_frame_fills_the_window_at_100_percent() {
    let mut s = Session::new();
    let (w, ..) = wide(&mut s);
    s.execute("proto.settings", &json!({"responsive": true})).unwrap();
    let mut u = Ui::new(s);
    u.run("proto.present", json!({"start": w}));
    let p = presenter(&u);
    // The window is 1440×900; the title and tab bars take 80.
    assert_eq!(size(&p["frameSize"]), (1440.0, 820.0), "{p}");
    assert_eq!(p["zoom"], json!(1.0));
    assert_eq!(size(&p["panRange"]), (0.0, 0.0));
    // Z has nothing to cycle: the size options don't apply.
    u.key(pos2(720.0, 470.0), Key::Z);
    assert_eq!(presenter(&u)["scale"], "fit");
    // The preview pane (after the presenter closes): the frame takes the pane's area.
    u.key(pos2(720.0, 470.0), Key::Escape);
    u.run("proto.preview", json!({"start": w}));
    let pane = u.pane();
    let (pw, ph) = size(&u.preview()["frameSize"]);
    assert!((pw - f64::from(pane.width())).abs() <= 4.0 && ph < f64::from(pane.height()) && ph > 100.0, "{pw}×{ph} in {pane:?}");
}

#[test]
fn custom_sizes_present_as_a_bare_screen() {
    let (mut u, a, ..) = setup();
    u.run("proto.settings", json!({"width": 500, "height": 700}));
    u.run("proto.present", json!({"start": a}));
    let p = presenter(&u);
    assert_eq!(p["device"], "Custom size");
    assert_eq!(p["deviceColor"], Value::Null, "no body");
    assert_eq!(size(&p["viewport"]), (393.0, 700.0), "the 393-wide frame clipped to the screen's height");
    assert_eq!(p["scrollRange"][1], json!(1300.0));
    // Responsive: the frame becomes the screen; its content scrolls through what's below.
    u.run("proto.settings", json!({"responsive": true}));
    assert_eq!(size(&presenter(&u)["frameSize"]), (500.0, 700.0));
}

#[test]
fn prototype_panel_shows_colours_custom_sizes_and_responsive() {
    let (mut u, ..) = setup();
    u.app.ui.right_tab = crate::RightTab::Prototype;
    u.run("select.set", json!({"ids": []}));
    for settings in [json!({"color": "Teal"}), json!({"device": "MacBook Air"}), json!({"width": 640, "height": 480}), json!({"responsive": true})] {
        u.run("proto.settings", settings);
        u.frames(2, Vec::new);
    }
    // Each drew; the last settings stand.
    let doc = u.app.session.doc().unwrap();
    let p = crate::present::page_settings(doc, doc.top_frames(u.app.session.page().unwrap())[0]);
    assert_eq!((p.device.as_str(), p.custom_size, p.responsive), ("Custom size", [640.0, 480.0], true));
}

/// A library file with a "Brand" collection (Light/Dark) saved under the temp dir.
fn brand_library(tag: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("figcraft-libmodes-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("brand.figcraft").to_string_lossy().into_owned();
    let mut lib = Session::new();
    let ex = |s: &mut Session, c: &str, p: Value| s.execute(c, &p).unwrap();
    ex(&mut lib, "file.rename", json!({"name": "Brand kit"}));
    ex(&mut lib, "variable.createCollection", json!({"name": "Brand", "modes": ["Light", "Dark"]}));
    ex(
        &mut lib,
        "variable.create",
        json!({"collection": "Brand", "name": "ink", "type": "color", "values": {"Light": "#000000", "Dark": "#ffffff"}}),
    );
    let lid = ex(&mut lib, "library.publish", json!({}))["id"].as_str().unwrap().to_string();
    ex(&mut lib, "file.saveAs", json!({"path": path}));
    (path, lid)
}

#[test]
fn stored_variable_ids_keep_library_mode_picker_rows() {
    let (path, lid) = brand_library("stored-ids");
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"width": 100, "height": 100})).unwrap());
    s.execute("variable.createCollection", &json!({"name": "shadow"})).unwrap();
    s.execute("library.add", &json!({"path": path})).unwrap();
    s.execute("library.setVariableMode", &json!({"library": lid, "collection": "Brand", "mode": "Dark", "ids": [f]})).unwrap();
    let local = s.doc().unwrap().libraries.collections[0].local.clone();
    s.execute("variable.renameCollection", &json!({"collection": "shadow", "name": local})).unwrap();
    let mut u = Ui::new(s);
    let doc = u.app.session.doc().unwrap();
    let libs = crate::variable_ui::libraries(&u.app, doc);
    // Both a read library and an unavailable one keep the actual copy's modes, not its namesake's.
    for available in [libs.as_slice(), &[]] {
        let rows = crate::variable_ui::mode_rows(doc, available);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].coll.id, local);
        assert_eq!(rows[0].coll.name, "Brand");
        assert_eq!(rows[0].coll.modes.len(), 2);
        assert_eq!(rows[0].local, Some(local.as_str()));
        assert_eq!(rows[0].library.unwrap().0, lid);
    }
    u.run("select.set", json!({"ids": [f]}));
    u.frames(2, Vec::new);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn library_collections_show_in_the_mode_picker_under_their_library() {
    let (path, lid) = brand_library("picker");
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap();
    s.execute("library.add", &json!({"path": path})).unwrap();
    let mut u = Ui::new(s);
    let rows = |u: &Ui| -> Vec<(String, Option<String>, bool)> {
        let doc = u.app.session.doc().unwrap();
        let libs = crate::variable_ui::libraries(&u.app, doc);
        crate::variable_ui::mode_rows(doc, &libs)
            .iter()
            .map(|r| (r.coll.name.clone(), r.library.map(|l| l.1.to_string()), r.local.is_some()))
            .collect()
    };
    let lib_name = u.app.session.doc().unwrap().libraries.enabled[0].name.clone();
    assert_eq!(rows(&u), [("Theme".into(), None, true), ("Brand".into(), Some(lib_name.clone()), false)]);
    // The Design panel draws the section for the selected frame.
    u.run("select.set", json!({"ids": [f]}));
    u.frames(2, Vec::new);
    // Picking a library mode copies the collection in; the row then holds the copy's modes.
    u.run("library.setVariableMode", json!({"library": lid, "collection": "Brand", "mode": "Dark", "ids": [f]}));
    assert_eq!(rows(&u), [("Theme".into(), None, true), ("Brand".into(), Some(lib_name.clone()), true)]);
    let doc = u.app.session.doc().unwrap();
    let local = doc.libraries.collections[0].local.clone();
    assert_eq!(doc.node(f).unwrap().var_modes.len(), 1);
    assert!(doc.node(f).unwrap().var_modes.contains_key(&local));
    u.frames(2, Vec::new);
    // The library unlinked: the copy stays listed under the library's name, set like this file's.
    u.run("library.remove", json!({"id": lid}));
    let r = rows(&u);
    assert_eq!(r.len(), 2, "{r:?}");
    assert!(r[1].2 && r[1].0 == "Brand", "{r:?}");
    u.frames(2, Vec::new);
    let _ = std::fs::remove_file(&path);
}
