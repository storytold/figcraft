//! A page's prototype background in `.fig` exports.

use figcraft_doc::color::Color;
use figcraft_doc::proto::{DeviceRotation, PrototypeSettings};
use figcraft_doc::{Document, NodeKind};

use super::kiwi::Kv;
use super::{Archive, decode_canvas, export, import};

fn blue() -> Color {
    Color::from_rgb8_const(0x33, 0x66, 0x99)
}

/// The background of the first page, and its device, after exporting and importing `doc`.
fn back(doc: &Document) -> (Color, String, Vec<String>) {
    let (bytes, warnings) = export(doc, None).unwrap();
    let (imported, _) = import(&bytes, "x").unwrap();
    let page = imported.first_page().and_then(|p| imported.node(p)).unwrap();
    let NodeKind::Page { prototype, .. } = &page.kind else { panic!("not a page") };
    (prototype.background, prototype.device.clone(), warnings)
}

/// The `prototypeBackgroundColor` (r, g, b, a) in the record the export writes for the page of `doc`.
fn written(doc: &Document) -> Option<[f32; 4]> {
    let (bytes, _) = export(doc, None).unwrap();
    let archive = Archive::open(&bytes).unwrap();
    let (schema, message) = decode_canvas(&archive.canvas).unwrap();
    let msg = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
    let page = msg.get("nodeChanges").unwrap().list().iter().find(|n| n.get("type").and_then(Kv::str) == Some("CANVAS")).unwrap();
    page.get("prototypeBackgroundColor").map(|c| ["r", "g", "b", "a"].map(|k| c.get(k).and_then(Kv::f64).unwrap() as f32))
}

fn with_prototype(set: impl FnOnce(&mut figcraft_doc::proto::PrototypeSettings)) -> Document {
    let mut doc = Document::new("t");
    let page = doc.first_page().unwrap();
    if let Some(NodeKind::Page { prototype, .. }) = doc.node_mut(page).map(|n| &mut n.kind) {
        set(prototype);
    }
    doc
}

#[test]
fn a_pages_prototype_background_survives_a_fig_export() {
    let doc = with_prototype(|p| p.background = blue());
    assert_eq!(written(&doc), Some([blue().r, blue().g, blue().b, 1.0]));
    let (background, _, warnings) = back(&doc);
    assert_eq!(background, blue());
    assert!(warnings.iter().all(|w| !w.contains("prototype")), "{warnings:?}");
}

#[test]
fn a_page_with_a_device_keeps_its_background_and_warns_for_the_device() {
    let doc = with_prototype(|p| {
        p.background = blue();
        p.device = "iPhone 17".into();
    });
    let (background, _, warnings) = back(&doc);
    assert_eq!(background.to_hex(), blue().to_hex());
    assert!(warnings.iter().any(|w| w.contains("prototype device settings were left out")), "{warnings:?}");
}

#[test]
fn a_default_page_writes_no_prototype_background_and_no_warning() {
    let doc = with_prototype(|_| {});
    assert_eq!(written(&doc), None);
    let (background, _, warnings) = back(&doc);
    assert_eq!(background.to_hex(), figcraft_doc::proto::PrototypeSettings::default().background.to_hex());
    assert!(warnings.iter().all(|w| !w.contains("prototype")), "{warnings:?}");
}

#[test]
fn the_schema_numbers_the_prototype_background() {
    let s = super::kiwi::Schema::parse(super::export::SCHEMA).unwrap();
    let def = |n: &str| s.defs.iter().find(|d| d.name == n).unwrap();
    let id = |f: &str| def("NodeChange").fields.iter().find(|x| x.name == f).unwrap().value;
    assert_eq!(id("prototypeBackgroundColor"), 141);
}

#[test]
fn each_device_setting_alone_warns_and_writes_no_background() {
    let settings: [fn(&mut PrototypeSettings); 5] = [
        |p| p.device = "iPhone 17".into(),
        |p| p.rotation = DeviceRotation::Landscape,
        |p| p.color = "White".into(),
        |p| p.custom_size = [320.0, 480.0],
        |p| p.responsive = true,
    ];
    for (i, set) in settings.into_iter().enumerate() {
        let doc = with_prototype(set);
        assert_eq!(written(&doc), None, "setting {i}");
        let (_, _, warnings) = back(&doc);
        assert!(warnings.iter().any(|w| w == "prototype device settings were left out"), "setting {i}: {warnings:?}");
    }
}

#[test]
fn a_prototype_background_is_written_opaque() {
    // The import reads an alpha of 0 as no background, and any other alpha as 1.
    for alpha in [0.0, 0.5] {
        let doc = with_prototype(|p| p.background = blue().with_alpha(alpha));
        assert_eq!(written(&doc), Some([blue().r, blue().g, blue().b, 1.0]), "alpha {alpha}");
        let (background, _, warnings) = back(&doc);
        assert_eq!(background, blue(), "alpha {alpha}");
        assert!(warnings.iter().any(|w| w == "a prototype background's opacity was left out"), "alpha {alpha}: {warnings:?}");
    }
    // The default colour is the default at any opacity: no field.
    let doc = with_prototype(|p| p.background = p.background.with_alpha(0.5));
    assert_eq!(written(&doc), None);
}
