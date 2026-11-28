//! #2189: the eyedropper picks the colour the canvas shows under the pointer. A magnified image
//! draws its pixels on the canvas (`magnified_image_pixels`), so the picked colour is that pixel's,
//! not a blend with its neighbours.

use base64::Engine as _;
use serde_json::json;

use crate::Session;
use crate::tools::{PointerEvent, PointerKind};

/// A 9 x 9 image of `back` with a 1 px vertical line of `line` in column 4 (a thin border in a
/// screenshot, as in the report).
fn line_png(back: [u8; 3], line: [u8; 3]) -> Vec<u8> {
    let mut p = figcraft_render::Pixmap::new(9, 9).unwrap();
    for (i, px) in p.data_mut().chunks_exact_mut(4).enumerate() {
        let c = if i % 9 == 4 { line } else { back };
        px.copy_from_slice(&[c[0], c[1], c[2], 255]);
    }
    figcraft_render::encode_png(&p).unwrap()
}

/// The colour the eyedropper picks at page point (x, y) at `zoom`.
fn picked(png: &[u8], x: f64, y: f64, zoom: f64) -> String {
    let mut s = Session::new();
    let data = base64::engine::general_purpose::STANDARD.encode(png);
    s.execute("image.place", &json!({"data": data, "x": 0, "y": 0})).unwrap();
    s.execute("edit.pickColor", &json!({"apply": false})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, x, y, zoom)).unwrap();
    s.tool.picked.map(|c| c.to_hex()).unwrap_or_default()
}

#[test]
fn the_eyedropper_picks_a_magnified_images_pixel_not_a_blend_with_its_neighbours() {
    // The reporter's colours: a #3D444D border, 1 px wide, beside a darker background.
    let png = line_png([0x15, 0x1B, 0x23], [0x3D, 0x44, 0x4D]);
    for zoom in [2.0, 8.0, 20.0] {
        assert_eq!(picked(&png, 4.5, 4.5, zoom), "3D444D", "on the line at {zoom}x");
        assert_eq!(picked(&png, 2.5, 4.5, zoom), "151B23", "beside it at {zoom}x");
    }
    // At 1:1 the canvas pixel is the image's pixel too.
    assert_eq!(picked(&png, 4.5, 4.5, 1.0), "3D444D", "on the line at 1x");
}
