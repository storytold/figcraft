//! A page's prototype background in `.fig` exports.

use figcraft_doc::color::Color;
use figcraft_doc::{Document, NodeKind};

use super::{export, import};

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
    let (background, _, warnings) = back(&doc);
    assert_eq!(background.to_hex(), blue().to_hex());
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
