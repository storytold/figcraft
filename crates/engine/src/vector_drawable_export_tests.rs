//! Android Vector Drawable (XML) export through the commands: one-off exports with their
//! warnings, export settings with a compatibility mode (stored, saved, edited), export layers,
//! and bad values refused.

use base64::Engine as _;
use figcraft_doc::*;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

/// A session with a 48 × 24 frame holding two overlapping rectangles at 50% opacity.
fn session() -> (Session, NodeId) {
    let mut s = Session::new();
    let r = s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 48, "height": 24, "name": "Badge"})).unwrap();
    let f = id_of(&r["id"]).unwrap();
    for x in [0, 12] {
        s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 24, "height": 24, "parent": f})).unwrap();
    }
    s.execute("node.set", &json!({"ids": [f], "props": {"opacity": 0.5}})).unwrap();
    s.set_selection(vec![f]);
    (s, f)
}

fn xml(r: &Value) -> String {
    let bytes = base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap()).unwrap();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn a_one_off_export_returns_the_drawable_and_its_warnings() {
    let (mut s, f) = session();
    let r = s.execute("file.export", &json!({"ids": [f], "format": "xml"})).unwrap();
    assert_eq!(r["format"], "xml");
    let x = xml(&r);
    assert!(x.starts_with("<?xml") && x.contains("<vector") && x.contains("android:width=\"48dp\""), "{x}");
    assert_eq!(r["warnings"], json!(["layer opacity was applied to each shape: where shapes overlap they look denser (Badge)"]));
    // Exact leaves the faded frame out; ignore draws it opaque, without warnings.
    let r = s.execute("file.export", &json!({"ids": [f], "format": "xml", "vdCompat": "exact"})).unwrap();
    assert!(!xml(&r).contains("fillColor"), "{}", xml(&r));
    // Still a valid (empty) drawable, and it says so.
    assert_eq!(r["warnings"].as_array().map(Vec::len), Some(2));
    assert_eq!(r["warnings"][1], "nothing in it could be drawn: the drawable is empty (Badge)");
    let r = s.execute("file.export", &json!({"ids": [f], "format": "xml", "vdCompat": "ignore"})).unwrap();
    assert_eq!(r["warnings"], json!([]));
    // Names only when asked.
    assert!(!xml(&r).contains("android:name"));
    let r = s.execute("file.export", &json!({"ids": [f], "format": "xml", "svgIdAttribute": true})).unwrap();
    assert!(xml(&r).contains("android:name=\"Badge\""), "{}", xml(&r));
    // Other formats don't grow a `warnings` field.
    assert!(s.execute("file.export", &json!({"ids": [f], "format": "svg"})).unwrap().get("warnings").is_none());
}

#[test]
fn a_path_ending_in_xml_picks_the_format() {
    let (mut s, f) = session();
    let dir = std::env::temp_dir().join(format!("figcraft-vd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("badge.xml");
    let r = s.execute("file.export", &json!({"ids": [f], "path": path.to_string_lossy()})).unwrap();
    assert_eq!(r["format"], "xml");
    assert!(std::fs::read_to_string(&path).unwrap().contains("<vector"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_settings_keep_their_compatibility_mode() {
    let (mut s, f) = session();
    s.execute("export.add", &json!({"ids": [f], "setting": {"format": "xml", "vdCompat": "exact"}})).unwrap();
    let e = s.doc().unwrap().node(f).unwrap().exports[0].clone();
    assert_eq!((e.format, e.vd_compat), (ExportFormat::VectorDrawable, VdCompat::Exact));
    // Stored camelCase, the default left out.
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!((v["format"].as_str(), v["vdCompat"].as_str()), (Some("vectorDrawable"), Some("exact")));
    assert!(serde_json::to_value(ExportSetting::default()).unwrap().get("vdCompat").is_none());
    // Saved and loaded.
    let bytes = figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    let (back, _) = figcraft_format::load_bytes(&bytes).unwrap();
    assert_eq!(back.node(f).unwrap().exports[0], e);
    // Edited and undone.
    s.execute("export.update", &json!({"ids": [f], "index": 0, "vdCompat": "ignore"})).unwrap();
    assert_eq!(s.doc().unwrap().node(f).unwrap().exports[0].vd_compat, VdCompat::Ignore);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().node(f).unwrap().exports[0].vd_compat, VdCompat::Exact);
    // Export layers: one .xml file with its warnings; a vdCompat passed there wins.
    let r = s.execute("file.exportLayers", &json!({"ids": [f]})).unwrap();
    let file = &r["files"][0];
    assert_eq!(file["name"], "Badge.xml");
    assert_eq!(file["warnings"].as_array().map(Vec::len), Some(2), "exact: left out, so empty");
    let r = s.execute("file.exportLayers", &json!({"ids": [f], "vdCompat": "ignore"})).unwrap();
    assert_eq!(r["files"][0]["warnings"], json!([]));
}

#[test]
fn bad_values_are_refused_and_change_nothing() {
    let (mut s, f) = session();
    s.execute("export.add", &json!({"ids": [f], "setting": {"format": "xml"}})).unwrap();
    for junk in [json!("loose"), json!(3), json!(true), json!("BESTEFFORT"), json!({})] {
        assert!(s.execute("file.export", &json!({"ids": [f], "format": "xml", "vdCompat": junk})).is_err(), "{junk}");
        assert!(s.execute("export.update", &json!({"ids": [f], "index": 0, "vdCompat": junk})).is_err(), "{junk}");
        assert!(s.execute("export.add", &json!({"ids": [f], "setting": {"vdCompat": junk}})).is_err(), "{junk}");
        assert!(s.execute("file.exportLayers", &json!({"ids": [f], "vdCompat": junk})).is_err(), "{junk}");
    }
    let n = s.doc().unwrap().node(f).unwrap();
    assert_eq!((n.exports.len(), n.exports[0].vd_compat), (1, VdCompat::BestEffort));
    let err = s.execute("file.export", &json!({"ids": [f], "format": "vml"})).unwrap_err().to_string();
    assert!(err.contains("xml"), "{err}");
}
