//! Design data whose image fill names an image it doesn't carry, as a clipboard copied from
//! another design app does (#1646): the fill keeps the image's name, so the image can be put back
//! under it; the layer shows the missing-image placeholder (light grey with darker stripes, not a
//! grey a design could have chosen); and the import says so, once per image. Built in code and
//! read as `edit.pasteDesignLayers` reads a clipboard's design data.

use figcraft_doc::Document;
use figcraft_doc::color::PaintKind;

use super::kiwi::Kv;
use super::tests::{arr, canvas, e, node, o, schema, xy};
use super::try_import;
use crate::rest::{is_missing_image, missing_image_placeholder};

const HASH: [u8; 20] = [0x5a; 20];

fn image_fill() -> Kv<'static> {
    o(vec![("type", e("IMAGE")), ("image", o(vec![("hash", Kv::Bytes(HASH.to_vec()))])), ("imageScaleMode", e("FILL"))])
}

/// Two layers filled with the same image, which the data doesn't carry.
fn pasted() -> (Document, Vec<String>) {
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
        node(10, Some((1, "!")), "RECTANGLE", "Hero", vec![("size", xy(200.0, 100.0)), ("fillPaints", arr(vec![image_fill()]))]),
        node(11, Some((1, "\"")), "RECTANGLE", "Hero copy", vec![("size", xy(200.0, 100.0)), ("fillPaints", arr(vec![image_fill()]))]),
    ];
    let bytes = canvas(&schema(), &o(vec![("nodeChanges", arr(nodes))]));
    try_import(&bytes, "Pasted layers").expect("design data").expect("it imports")
}

#[test]
fn a_missing_image_keeps_its_name_and_shows_as_missing() {
    let (doc, warnings) = pasted();
    let hex: String = HASH.iter().map(|b| format!("{b:02x}")).collect();
    for name in ["Hero", "Hero copy"] {
        let layer = doc.nodes().find(|n| n.name == name).unwrap();
        let Some(PaintKind::Image(img)) = layer.fills.first().map(|p| &p.kind) else { panic!("{name}: {:?}", layer.fills) };
        assert_eq!(img.image, hex, "{name}: the image's own name, so it can come back under it");
        assert!(doc.images.get(&img.image).is_some_and(is_missing_image), "{name}: the missing-image placeholder");
    }
    let note = "image fills whose image isn't in the file show a missing-image placeholder and keep its name";
    assert_eq!(warnings.iter().filter(|w| w.starts_with(note)).count(), 1, "{warnings:?}");
    assert!(!warnings.iter().any(|w| w.contains("without image data")), "the REST JSON note isn't given too: {warnings:?}");
}

#[test]
fn the_placeholder_reads_as_missing_not_as_grey() {
    let png = image::load_from_memory(missing_image_placeholder().unwrap()).unwrap().to_rgba8();
    // Two tones (their anti-aliased edges between): darker stripes over a light ground, not one
    // flat grey.
    let count = |c: u8| png.pixels().filter(|p| p.0 == [c, c, c, 0xFF]).count();
    let all = png.pixels().count();
    let (ground, stripe) = (count(0xE6), count(0xB4));
    assert!(ground > all / 2 && stripe > all / 8 && ground + stripe > all * 3 / 4, "{ground} ground, {stripe} stripe of {all}");
    // A real image of one colour is never taken for the placeholder.
    let flat = figcraft_doc::ImageData { bytes: std::sync::Arc::new(vec![1, 2, 3]), width: 64, height: 64 };
    assert!(!is_missing_image(&flat));
}
