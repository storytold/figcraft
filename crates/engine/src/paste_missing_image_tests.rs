//! Layers pasted from another design app whose image the clipboard doesn't carry (#1646): the
//! paste names them (`missingImages`), and they keep their image's name. Design data made by our
//! own `.fig` export: its canvas alone, as a clipboard carries it, and the whole file with images.

use std::io::Read;

use base64::Engine as _;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

/// A `.fig` export of a page with "Photo" (an image fill) and "Plain" (a solid fill).
fn exported() -> Vec<u8> {
    let mut s = Session::new();
    let photo = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 80, "height": 60})).unwrap()["id"]).unwrap();
    let plain = id_of(&s.execute("shape.rectangle", &json!({"x": 100, "y": 0, "width": 80, "height": 60})).unwrap()["id"]).unwrap();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 40, 40, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let doc = s.doc_mut().unwrap();
    let (hash, _, _) = figcraft_format::add_image(doc, png).unwrap();
    let n = doc.get_mut(photo).unwrap();
    n.name = "Photo".into();
    n.fills = vec![figcraft_color::Paint::image(hash, figcraft_color::ImageScaleMode::Fill)];
    doc.get_mut(plain).unwrap().name = "Plain".into();
    let r = s.execute("file.export", &json!({"format": "fig"})).unwrap();
    base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap()).unwrap()
}

/// Just the canvas of a `.fig` file: the design data a clipboard carries, without the images.
fn canvas_only(fig: &[u8]) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(fig)).unwrap();
    let mut out = Vec::new();
    z.by_name("canvas.fig").unwrap().read_to_end(&mut out).unwrap();
    out
}

fn paste(data: &[u8]) -> Value {
    let mut s = Session::new();
    s.execute("edit.pasteDesignLayers", &json!({"data": base64::engine::general_purpose::STANDARD.encode(data)})).unwrap()
}

#[test]
fn layers_whose_image_the_clipboard_lacks_are_named() {
    let fig = exported();
    let r = paste(&canvas_only(&fig));
    assert_eq!(r["missingImages"], json!(["Photo"]), "{r}");
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("missing-image placeholder")), "{r}");
    // With its images, nothing is missing.
    let r = paste(&fig);
    assert!(r.get("missingImages").is_none(), "{r}");
}
