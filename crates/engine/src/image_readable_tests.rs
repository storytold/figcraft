//! An image file whose header reads but whose pixels don't is refused with a reason (#1474). It
//! used to be accepted and show as an empty box, with nothing said: a PNG cut off before its end
//! marker, or with damaged pixel data, looks fine until it is decoded.

use std::io::Cursor;

use base64::Engine as _;
use figcraft_doc::{NodeId, NodeKind};
use serde_json::{Value, json};

use crate::{Session, params};

/// A valid opaque red `w` × `h` PNG.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([255, 0, 0, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// `bytes` as `image.place` takes them.
fn data(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn place(s: &mut Session, bytes: &[u8], extra: Value) -> crate::Result<Value> {
    let mut p = json!({"data": data(bytes)});
    p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    s.execute("image.place", &p)
}

/// The ways a PNG with a good header can be damaged.
fn damaged() -> Vec<(&'static str, Vec<u8>)> {
    let good = png(16, 16);
    let mut corrupt = good.clone();
    // Past the signature and the header chunk, inside the pixel data.
    for b in corrupt.iter_mut().skip(41).take(8) {
        *b = 0;
    }
    vec![("no end marker", good[..good.len() - 12].to_vec()), ("cut in the pixels", good[..43].to_vec()), ("damaged pixels", corrupt)]
}

fn refused(r: crate::Result<Value>, what: &str) {
    let e = r.expect_err(what).to_string();
    assert!(e.contains("can't be read"), "{what}: {e}");
}

#[test]
fn a_damaged_image_is_refused_with_a_reason() {
    for (what, bytes) in damaged() {
        let mut s = Session::new();
        let nodes = s.doc().unwrap().nodes().count();
        refused(place(&mut s, &bytes, json!({})), what);
        let doc = s.doc().unwrap();
        assert_eq!(doc.nodes().count(), nodes, "{what}: no empty box");
        assert!(doc.images.is_empty(), "{what}: nothing kept");
    }
}

#[test]
fn a_damaged_image_doesnt_fill_a_layer() {
    for (what, bytes) in damaged() {
        let mut s = Session::new();
        let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
        let id: NodeId = params::id_of(&r["id"]).unwrap();
        let fills = s.doc().unwrap().node(id).unwrap().fills.clone();
        refused(place(&mut s, &bytes, json!({"target": id})), what);
        assert_eq!(s.doc().unwrap().node(id).unwrap().fills, fills, "{what}");
    }
}

/// Opening a damaged image as a file of its own says so too.
#[test]
fn a_damaged_image_doesnt_replace_an_image() {
    for (what, bytes) in damaged() {
        let mut s = Session::new();
        let id = place(&mut s, &png(4, 4), json!({})).unwrap()["id"].clone();
        let before = s.doc().unwrap().images.len();
        refused(s.execute("image.replace", &json!({"ids": [id], "data": data(&bytes)})), what);
        assert_eq!(s.doc().unwrap().images.len(), before, "{what}: nothing kept");
    }
}

#[test]
fn a_damaged_image_doesnt_open_as_a_blank_file() {
    for (what, bytes) in damaged() {
        let mut s = Session::new();
        let open = s.execute("file.open", &json!({"name": "damaged.png", "data": data(&bytes)}));
        assert!(open.is_err(), "{what}");
    }
    let mut s = Session::new();
    s.execute("file.open", &json!({"name": "good.png", "data": data(&png(16, 16))})).unwrap();
    assert!(s.doc().unwrap().nodes().any(|n| matches!(n.kind, NodeKind::Rectangle)));
}

/// A sound image goes in as ever, and refusing a damaged copy of something leaves what's there.
#[test]
fn a_sound_image_is_placed_and_stays() {
    let mut s = Session::new();
    let good = png(16, 16);
    let r = place(&mut s, &good, json!({})).unwrap();
    assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(16.0), Some(16.0)));
    let again = place(&mut s, &good, json!({})).unwrap();
    assert_eq!(again["image"], r["image"], "the same bytes are the same image");
    assert_eq!(s.doc().unwrap().images.len(), 1);
    refused(place(&mut s, &damaged()[0].1, json!({})), "damaged");
    assert_eq!(s.doc().unwrap().images.len(), 1, "the sound image is kept");
    // What isn't an image at all keeps its own message.
    assert!(place(&mut s, b"not an image", json!({})).is_err());
}
