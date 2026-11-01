use std::sync::Arc;

use figcraft_color::{ImageScaleMode, Paint};
use figcraft_doc::{Document, ImageData, NodeId, NodeKind};

use crate::{Options, Pixmap, Retained, View, encode_png, render_view};

fn checkerboard_doc() -> (Document, NodeId) {
    let mut img = Pixmap::new(4, 4).unwrap();
    for (i, c) in img.data_mut().chunks_exact_mut(4).enumerate() {
        let black = (i % 4 + i / 4) % 2 == 0;
        c.copy_from_slice(if black { &[0, 0, 0, 255] } else { &[255, 255, 255, 255] });
    }
    let mut d = Document::new("magnified");
    let page = d.first_page().unwrap();
    d.images.insert("checker".into(), ImageData { bytes: Arc::new(encode_png(&img).unwrap()), width: 4, height: 4 });
    let r = d.create(page, NodeKind::Rectangle, "Image", 0.0, 0.0, 4.0, 4.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::image("checker", ImageScaleMode::Fill)];
    (d, page)
}

fn view(scale: f64) -> View {
    let side = (4.0 * scale).round() as u32;
    View { origin: (0.0, 0.0), scale, width: side, height: side }
}

fn canvas() -> Options {
    Options { lod: true, magnified_image_pixels: true, ..Default::default() }
}

fn in_between_greys(p: &Pixmap, side: u32) -> usize {
    let row = p.width() as usize * 4;
    p.data().chunks_exact(row).take(side as usize).flat_map(|r| r[..side as usize * 4].chunks_exact(4)).filter(|c| c[0] > 8 && c[0] < 247).count()
}

#[test]
fn a_magnified_image_on_the_canvas_shows_its_pixels_as_hard_edged_squares() {
    let (d, page) = checkerboard_doc();
    let v = view(8.0);
    let pix = render_view(&d, page, &v, &canvas()).unwrap();
    assert_eq!(in_between_greys(&pix, v.width), 0, "each image pixel is one flat 8 × 8 square");
    for (x, y, black) in [(3, 3, true), (11, 3, false), (3, 11, false), (11, 11, true), (15, 20, false)] {
        let i = ((y * v.width + x) * 4) as usize;
        assert_eq!(pix.data()[i] == 0, black, "({x}, {y})");
    }
}

#[test]
fn exports_keep_magnified_images_smooth() {
    let (d, page) = checkerboard_doc();
    let v = view(8.0);
    let pix = render_view(&d, page, &v, &Options::default()).unwrap();
    assert!(in_between_greys(&pix, v.width) > 0, "an export scaled up blends neighbouring pixels");
}

#[test]
fn below_twice_its_size_an_image_stays_smooth_on_the_canvas() {
    let (d, page) = checkerboard_doc();
    let v = view(1.5);
    let canvas = render_view(&d, page, &v, &canvas()).unwrap();
    let export = render_view(&d, page, &v, &Options::default()).unwrap();
    assert_eq!(canvas.data(), export.data());
}

#[test]
fn turning_pixel_squares_on_redraws_a_retained_view() {
    let (d, page) = checkerboard_doc();
    let d = Arc::new(d);
    let v = view(8.0);
    let mut r = Retained::default();
    let smooth = r.render(&d, page, &v, &Options { lod: true, ..Default::default() }).unwrap().1.clone();
    let squares = r.render(&d, page, &v, &canvas()).unwrap().1.clone();
    assert!(in_between_greys(&smooth, v.width) > 0);
    assert_eq!(in_between_greys(&squares, v.width), 0);
}
