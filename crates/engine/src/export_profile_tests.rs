//! The color profile an export setting writes PNG, JPG and WebP files in: same as the file (the
//! default, following the document's profile when it exports), sRGB or Display P3, with the pixels
//! converted and the file tagged; stored per setting, kept across a format switch, passed to
//! export layers, ignored by the always-sRGB SVG, PDF and XML, and bad values refused.

use base64::Engine as _;
use figcraft_color::{Color, ColorProfile};
use figcraft_doc::*;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

/// A session with a red 20 × 10 rectangle in an sRGB document.
fn session() -> (Session, NodeId) {
    let mut s = Session::new();
    let r = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 10, "name": "Red"})).unwrap()["id"]).unwrap();
    s.execute("fill.set", &json!({"ids": [r], "color": "#FF0000"})).unwrap();
    (s, r)
}

fn bytes(v: &Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(v["data"].as_str().unwrap()).unwrap()
}

fn export(s: &mut Session, params: Value) -> Vec<u8> {
    bytes(&s.execute("file.export", &params).unwrap())
}

/// Whether `bytes` holds a PNG chunk of `kind` (with `data`, when given).
fn has_chunk(bytes: &[u8], kind: &[u8; 4], data: Option<&[u8]>) -> bool {
    bytes.windows(4).enumerate().any(|(i, w)| w == kind && data.is_none_or(|d| bytes.get(i + 4..i + 4 + d.len()) == Some(d)))
}

/// An embedded ICC display profile (its device class).
fn has_icc(bytes: &[u8]) -> bool {
    bytes.windows(4).any(|w| w == b"mntr")
}

/// The middle pixel's RGB.
fn middle(bytes: &[u8]) -> [u8; 3] {
    let img = image::load_from_memory(bytes).unwrap().to_rgba8();
    let [r, g, b, _] = img.get_pixel(img.width() / 2, img.height() / 2).0;
    [r, g, b]
}

/// `c` as 8-bit RGB.
fn rgb8(c: Color) -> [u8; 3] {
    [c.r, c.g, c.b].map(|v| (v * 255.0).round() as u8)
}

fn close(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tolerance)
}

/// sRGB red in Display P3 values.
fn red_in_p3() -> [u8; 3] {
    rgb8(ColorProfile::Srgb.conversion_to(ColorProfile::DisplayP3).apply(Color::new(1.0, 0.0, 0.0, 1.0)))
}

#[test]
fn a_display_p3_png_from_an_srgb_file_is_converted_and_tagged() {
    let (mut s, r) = session();
    let png = export(&mut s, json!({"ids": [r], "format": "png", "colorProfile": "displayP3"}));
    assert!(has_chunk(&png, b"iCCP", None) && has_chunk(&png, b"cICP", Some(&[12, 13, 0, 1])) && !has_chunk(&png, b"sRGB", None));
    let want = red_in_p3();
    assert!(close(middle(&png), want, 1), "{:?} vs {want:?}", middle(&png));
    // The default stays the file's own sRGB.
    let png = export(&mut s, json!({"ids": [r], "format": "png"}));
    assert!(has_chunk(&png, b"sRGB", None) && !has_chunk(&png, b"iCCP", None));
    assert_eq!(middle(&png), [255, 0, 0]);
    // The document is not changed by the export.
    assert_eq!(s.doc().unwrap().color_profile, ColorProfile::Srgb);
    assert_eq!(crate::paint_tests::fill(&s, r).to_hex(), "FF0000");
}

#[test]
fn an_srgb_export_of_a_display_p3_file_converts_back() {
    let (mut s, r) = session();
    s.execute("file.colorProfile", &json!({"profile": "displayP3"})).unwrap();
    let same = export(&mut s, json!({"ids": [r], "format": "png"}));
    assert!(has_chunk(&same, b"iCCP", None));
    assert!(close(middle(&same), red_in_p3(), 1), "{:?}", middle(&same));
    for p in ["srgb", "displayP3", "sameAsFile"] {
        let png = export(&mut s, json!({"ids": [r], "format": "png", "colorProfile": p}));
        assert_eq!(has_chunk(&png, b"sRGB", None), p == "srgb", "{p}");
        let want = if p == "srgb" { [255, 0, 0] } else { red_in_p3() };
        assert!(close(middle(&png), want, 1), "{p}: {:?} vs {want:?}", middle(&png));
    }
    // A wide-gamut color clips in sRGB and keeps its values in Display P3.
    s.execute("fill.set", &json!({"ids": [r], "color": "#00FF00"})).unwrap();
    let srgb = export(&mut s, json!({"ids": [r], "format": "png", "colorProfile": "srgb"}));
    let p3 = export(&mut s, json!({"ids": [r], "format": "png", "colorProfile": "displayP3"}));
    assert_eq!(middle(&p3), [0, 255, 0]);
    let [red, green, blue] = middle(&srgb);
    assert!(red == 0 && green == 255 && blue == 0, "P3 green clips to sRGB green: {:?}", middle(&srgb));
}

#[test]
fn jpg_and_webp_follow_the_setting() {
    let (mut s, r) = session();
    for f in ["jpg", "webp"] {
        // sRGB files stay untagged (read as sRGB); Display P3 files embed the generated profile.
        assert!(!has_icc(&export(&mut s, json!({"ids": [r], "format": f}))), "{f}");
        let p3 = export(&mut s, json!({"ids": [r], "format": f, "colorProfile": "displayP3"}));
        assert!(has_icc(&p3), "{f}");
        let want = red_in_p3();
        assert!(close(middle(&p3), want, 8), "{f}: {:?} vs {want:?}", middle(&p3));
    }
    s.execute("file.colorProfile", &json!({"profile": "displayP3"})).unwrap();
    for f in ["jpg", "webp"] {
        let srgb = export(&mut s, json!({"ids": [r], "format": f, "colorProfile": "srgb"}));
        assert!(!has_icc(&srgb), "{f}");
        assert!(close(middle(&srgb), [255, 0, 0], 8), "{f}: {:?}", middle(&srgb));
    }
}

#[test]
fn a_translucent_jpg_is_flattened_before_its_conversion() {
    let (mut s, r) = session();
    s.execute("fill.set", &json!({"ids": [r], "opacity": 0.5})).unwrap();
    // The sRGB document shows 50% red over white; the Display P3 JPG holds that color converted.
    let flat = export(&mut s, json!({"ids": [r], "format": "jpg", "colorProfile": "srgb"}));
    let [fr, fg, fb] = middle(&flat);
    let shown = Color::new(f32::from(fr) / 255.0, f32::from(fg) / 255.0, f32::from(fb) / 255.0, 1.0);
    let want = rgb8(ColorProfile::Srgb.conversion_to(ColorProfile::DisplayP3).apply(shown));
    let p3 = export(&mut s, json!({"ids": [r], "format": "jpg", "colorProfile": "displayP3"}));
    assert!(close(middle(&p3), want, 2), "{:?} vs {want:?} (sRGB {:?})", middle(&p3), [fr, fg, fb]);
}

#[test]
fn svg_pdf_and_xml_stay_srgb() {
    let (mut s, r) = session();
    // A Display P3 file: its red converts back to sRGB #FF0000 whatever the setting says.
    s.execute("file.colorProfile", &json!({"profile": "displayP3"})).unwrap();
    for p in ["displayP3", "sameAsFile", "srgb"] {
        let svg = String::from_utf8(export(&mut s, json!({"ids": [r], "format": "svg", "colorProfile": p}))).unwrap().to_ascii_uppercase();
        assert!(svg.contains("#FF0000"), "{p}: {svg}");
        let xml = String::from_utf8(export(&mut s, json!({"ids": [r], "format": "xml", "colorProfile": p}))).unwrap().to_ascii_uppercase();
        assert!(xml.contains("#FF0000"), "{p}: {xml}");
        let pdf = export(&mut s, json!({"ids": [r], "format": "pdf", "colorProfile": p}));
        assert!(pdf.starts_with(b"%PDF") && !has_icc(&pdf), "{p}");
    }
}

#[test]
fn same_as_file_follows_the_document_and_a_choice_stays() {
    let (mut s, r) = session();
    s.execute("export.add", &json!({"ids": [r]})).unwrap();
    s.execute("export.add", &json!({"ids": [r], "setting": {"suffix": "-srgb", "colorProfile": "srgb"}})).unwrap();
    s.execute("export.add", &json!({"ids": [r], "setting": {"suffix": "-p3", "colorProfile": "displayP3"}})).unwrap();
    let files = |s: &mut Session| -> Vec<(String, bool)> {
        let v = s.execute("file.exportLayers", &json!({"ids": [r]})).unwrap();
        v["files"].as_array().unwrap().iter().map(|f| (f["name"].as_str().unwrap().to_string(), has_chunk(&bytes(f), b"iCCP", None))).collect()
    };
    let names = |icc: [bool; 3]| -> Vec<(String, bool)> {
        ["Red.png", "Red-srgb.png", "Red-p3.png"].into_iter().zip(icc).map(|(n, i)| (n.to_string(), i)).collect()
    };
    assert_eq!(files(&mut s), names([false, false, true]));
    s.execute("file.colorProfile", &json!({"profile": "displayP3"})).unwrap();
    assert_eq!(files(&mut s), names([true, false, true]));
    // A profile passed to export layers stands in for every setting's own.
    let v = s.execute("file.exportLayers", &json!({"ids": [r], "colorProfile": "srgb"})).unwrap();
    assert!(v["files"].as_array().unwrap().iter().all(|f| has_chunk(&bytes(f), b"sRGB", None)), "{v}");
}

#[test]
fn export_settings_keep_their_color_profile() {
    let (mut s, r) = session();
    s.execute("export.add", &json!({"ids": [r], "setting": {"colorProfile": "displayP3"}})).unwrap();
    let e = s.doc().unwrap().node(r).unwrap().exports[0].clone();
    assert_eq!(e.color_profile, ExportColorProfile::DisplayP3);
    // Stored camelCase, the default left out, and a row saved without it reads as same as file.
    assert_eq!(serde_json::to_value(&e).unwrap()["colorProfile"], "displayP3");
    assert!(serde_json::to_value(ExportSetting::default()).unwrap().get("colorProfile").is_none());
    let old: ExportSetting = serde_json::from_value(json!({"size": {"by": "scale", "value": 2.0}, "suffix": "@2x", "format": "jpg"})).unwrap();
    assert_eq!(old.color_profile, ExportColorProfile::SameAsFile);
    // Saved and loaded.
    let saved = figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    let (back, _) = figcraft_format::load_bytes(&saved).unwrap();
    assert_eq!(back.node(r).unwrap().exports[0], e);
    // Edited and undone; a format switch keeps it.
    s.execute("export.update", &json!({"ids": [r], "index": 0, "colorProfile": "srgb"})).unwrap();
    assert_eq!(s.doc().unwrap().node(r).unwrap().exports[0].color_profile, ExportColorProfile::Srgb);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().node(r).unwrap().exports[0].color_profile, ExportColorProfile::DisplayP3);
    for f in ["svg", "pdf", "png"] {
        s.execute("export.update", &json!({"ids": [r], "index": 0, "format": f})).unwrap();
        assert_eq!(s.doc().unwrap().node(r).unwrap().exports[0].color_profile, ExportColorProfile::DisplayP3, "{f}");
    }
    // `null` changes nothing.
    s.execute("export.update", &json!({"ids": [r], "index": 0, "colorProfile": null})).unwrap();
    assert_eq!(s.doc().unwrap().node(r).unwrap().exports[0].color_profile, ExportColorProfile::DisplayP3);
    for p in ExportColorProfile::ALL {
        assert_eq!(ExportColorProfile::from_key(p.key()), Some(p));
    }
}

#[test]
fn bad_color_profiles_are_refused_and_change_nothing() {
    let (mut s, r) = session();
    s.execute("export.add", &json!({"ids": [r]})).unwrap();
    for junk in [json!("cmyk"), json!(3), json!(true), json!("SRGB"), json!({})] {
        assert!(s.execute("file.export", &json!({"ids": [r], "colorProfile": junk})).is_err(), "{junk}");
        assert!(s.execute("export.update", &json!({"ids": [r], "index": 0, "colorProfile": junk})).is_err(), "{junk}");
        assert!(s.execute("export.add", &json!({"ids": [r], "setting": {"colorProfile": junk}})).is_err(), "{junk}");
        assert!(s.execute("file.exportLayers", &json!({"ids": [r], "colorProfile": junk})).is_err(), "{junk}");
    }
    let n = s.doc().unwrap().node(r).unwrap();
    assert_eq!((n.exports.len(), n.exports[0].color_profile), (1, ExportColorProfile::SameAsFile));
    let err = s.execute("file.export", &json!({"ids": [r], "colorProfile": "p3"})).unwrap_err().to_string();
    assert!(err.contains("sameAsFile|srgb|displayP3"), "{err}");
}
