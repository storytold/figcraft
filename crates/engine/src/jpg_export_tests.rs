//! JPG exports: the quality an export or export setting asks for, and the colour profile every
//! JPG embeds (#2004).

use image::ImageDecoder;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_color::ColorProfile;
use figcraft_doc::{ExportSetting, NodeId};

/// A rectangle with a gradient, so the quality shows in the file's size.
fn gradient_rect(s: &mut Session) -> NodeId {
    let r = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 200, "height": 120})).unwrap()["id"]).unwrap();
    let fill = json!({"type": "linearGradient", "stops": ["#FF0000", "#00FF88", "#2040FF"]});
    s.execute("node.set", &json!({"ids": [r], "props": {"fills": [fill]}})).unwrap();
    r
}

fn bytes(v: &Value) -> Vec<u8> {
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, v["data"].as_str().unwrap()).unwrap()
}

fn jpg(s: &mut Session, r: NodeId, extra: Value) -> Vec<u8> {
    let mut p = json!({"ids": [r], "format": "jpg"});
    if let (Value::Object(m), Value::Object(e)) = (&mut p, extra) {
        m.extend(e);
    }
    bytes(&s.execute("file.export", &p).unwrap())
}

fn icc(jpg: &[u8]) -> Option<Vec<u8>> {
    image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(jpg)).unwrap().icc_profile().unwrap()
}

#[test]
fn jpg_quality_is_taken_and_defaults_to_90() {
    let mut s = Session::new();
    let r = gradient_rect(&mut s);
    let default = jpg(&mut s, r, json!({}));
    assert_eq!(default, jpg(&mut s, r, json!({"quality": 90})), "90 is the default");
    let low = jpg(&mut s, r, json!({"quality": 20}));
    let high = jpg(&mut s, r, json!({"quality": 100}));
    assert!(low.len() < default.len() && default.len() < high.len(), "{} {} {}", low.len(), default.len(), high.len());
    // Agents send numbers as strings too.
    assert_eq!(jpg(&mut s, r, json!({"quality": "20"})), low);
    image::load_from_memory(&low).unwrap();
    // The other formats ignore it.
    let png = |s: &mut Session, extra: Value| {
        let mut p = json!({"ids": [r], "format": "png"});
        if let (Value::Object(m), Value::Object(e)) = (&mut p, extra) {
            m.extend(e);
        }
        bytes(&s.execute("file.export", &p).unwrap())
    };
    assert_eq!(png(&mut s, json!({"quality": 20})), png(&mut s, json!({})));
}

#[test]
fn bad_qualities_are_refused() {
    let mut s = Session::new();
    let r = gradient_rect(&mut s);
    s.execute("export.add", &json!({"ids": [r], "setting": {"format": "jpg"}})).unwrap();
    for q in [json!(0), json!(101), json!(-5), json!(50.5), json!("high"), json!(true), json!([80]), json!(1e300)] {
        let e = s.execute("file.export", &json!({"ids": [r], "format": "jpg", "quality": q})).unwrap_err();
        assert!(e.to_string().contains("quality: a whole number from 1 to 100"), "{q}: {e}");
        assert!(s.execute("export.add", &json!({"ids": [r], "setting": {"format": "jpg", "quality": q}})).is_err(), "{q}");
        assert!(s.execute("export.update", &json!({"ids": [r], "index": 0, "quality": q})).is_err(), "{q}");
    }
    let n = s.doc().unwrap().node(r).unwrap();
    assert_eq!(n.exports.len(), 1);
    assert_eq!(n.exports[0].quality, ExportSetting::DEFAULT_QUALITY);
    // null leaves the setting as it is.
    s.execute("export.update", &json!({"ids": [r], "index": 0, "quality": 40})).unwrap();
    s.execute("export.update", &json!({"ids": [r], "index": 0, "quality": null})).unwrap();
    assert_eq!(s.doc().unwrap().node(r).unwrap().exports[0].quality, 40);
}

#[test]
fn a_bad_quality_is_refused_whatever_the_format() {
    // As `vdCompat`, `bleed` and the SVG flags are: a value is checked for every format, so a
    // setting that becomes a JPG later never holds one the encoder can't take.
    let mut s = Session::new();
    let r = gradient_rect(&mut s);
    assert!(s.execute("file.export", &json!({"ids": [r], "format": "png", "vdCompat": "junk"})).is_err());
    let png = std::env::temp_dir().join(format!("figcraft-bad-quality-{}.png", std::process::id()));
    for mut p in [json!({"format": "png"}), json!({"format": "webp"}), json!({"format": "svg"}), json!({"path": png.to_string_lossy()})] {
        p["ids"] = json!([r]);
        p["quality"] = json!(0);
        let e = s.execute("file.export", &p).unwrap_err();
        assert!(e.to_string().contains("quality: a whole number from 1 to 100"), "{p}: {e}");
    }
    assert!(!png.exists(), "the refused export wrote nothing");
    // A valid quality on a PNG setting is kept, and used once the setting is a JPG.
    s.execute("export.add", &json!({"ids": [r], "setting": {"format": "png", "quality": 40}})).unwrap();
    s.execute("export.update", &json!({"ids": [r], "index": 0, "format": "jpg"})).unwrap();
    let files = s.execute("file.exportLayers", &json!({"ids": [r]})).unwrap()["files"].as_array().unwrap().clone();
    assert_eq!(bytes(&files[0]), jpg(&mut s, r, json!({"quality": 40})));
}

#[test]
fn export_settings_keep_their_quality() {
    let mut s = Session::new();
    let r = gradient_rect(&mut s);
    s.execute("export.add", &json!({"ids": [r], "setting": {"format": "jpg", "quality": 30}})).unwrap();
    s.execute("export.add", &json!({"ids": [r], "setting": {"format": "jpg", "suffix": "-hq"}})).unwrap();
    s.execute("export.update", &json!({"ids": [r], "index": 1, "quality": 95})).unwrap();
    let exports = s.doc().unwrap().node(r).unwrap().exports.clone();
    assert_eq!((exports[0].quality, exports[1].quality), (30, 95));
    // `file.exportLayers` encodes each setting at its own quality.
    let files = s.execute("file.exportLayers", &json!({"ids": [r]})).unwrap()["files"].as_array().unwrap().clone();
    assert_eq!(bytes(&files[0]), jpg(&mut s, r, json!({"quality": 30})));
    assert_eq!(bytes(&files[1]), jpg(&mut s, r, json!({"quality": 95})));
    // Saved only when it isn't the default, so older files and settings read as 90.
    assert_eq!(serde_json::to_value(&exports[0]).unwrap()["quality"], 30);
    assert!(serde_json::to_value(ExportSetting::default()).unwrap().get("quality").is_none());
    let old: ExportSetting = serde_json::from_value(json!({"format": "jpg"})).unwrap();
    assert_eq!(old.quality, ExportSetting::DEFAULT_QUALITY);
    // A value out of range in a file is clamped when it encodes.
    let odd: ExportSetting = serde_json::from_value(json!({"format": "jpg", "quality": 0})).unwrap();
    assert_eq!(odd.quality(), 1);
    // Undo takes the setting's quality back.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().node(r).unwrap().exports[1].quality, ExportSetting::DEFAULT_QUALITY);
}

#[test]
fn every_jpg_embeds_the_file_colour_profile() {
    for profile in ColorProfile::ALL {
        let mut s = Session::new();
        s.execute("file.colorProfile", &json!({"profile": profile.label(), "convert": false})).unwrap();
        let r = gradient_rect(&mut s);
        for quality in [json!({}), json!({"quality": 10})] {
            assert_eq!(icc(&jpg(&mut s, r, quality)), Some(profile.icc()), "{profile:?}");
        }
    }
}
