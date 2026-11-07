//! Large images on the canvas (#1483): a large image shown smaller than its pixels is drawn from a
//! copy no larger than it shows, so two of them stay decoded together, while exports and close-up
//! views draw from the image itself.

use std::sync::Arc;

use figcraft_color::{ImageScaleMode, Paint};
use figcraft_doc::{Document, ImageData, NodeId, NodeKind};
use tiny_skia::Pixmap;

use crate::images::{self, DISPLAY_SIDES, MAX_CACHED_BYTES};
use crate::{Options, View, render_view};

const W: u32 = 6000;
const H: u32 = 8;

/// A `W` × `H` PNG of one-pixel black and white columns, black first unless `phase` is odd.
fn stripes(phase: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(W, H, |x, _| image::Rgba(if (x + phase).is_multiple_of(2) { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// Two `W` × `H` images at their own size, one under the other (keys starting with `tag`: the
/// cache is shared by every test).
fn doc(tag: &str) -> (Document, NodeId, [String; 2]) {
    let mut d = Document::new("large images");
    let page = d.first_page().unwrap();
    let keys = [format!("{tag}-a"), format!("{tag}-b")];
    for (i, key) in (0u32..).zip(&keys) {
        d.images.insert(key.clone(), ImageData { bytes: Arc::new(stripes(i)), width: W, height: H });
        let r = d.create(page, NodeKind::Rectangle, "Image", 0.0, f64::from(i * 2 * H), f64::from(W), f64::from(H)).unwrap();
        d.get_mut(r).unwrap().fills = vec![Paint::image(key, ImageScaleMode::Fill)];
    }
    (d, page, keys)
}

fn canvas() -> Options {
    Options { lod: true, magnified_image_pixels: true, ..Default::default() }
}

/// The first `W` units of both rows at `scale`.
fn view(scale: f64) -> View {
    View { origin: (0.0, 0.0), scale, width: (f64::from(W) * scale).ceil() as u32, height: (f64::from(4 * H) * scale).ceil() as u32 }
}

/// The canvas's copy of image `key` of `d` shown `side` device pixels long.
fn shown(d: &Document, key: &str, side: f64) -> Arc<Pixmap> {
    images::shown(key, d.images.get(key), &Default::default(), d.color_profile, side).unwrap()
}

/// Whether row `y` of `p` shows one-pixel columns, black at even `x` when `black_first`, `each`
/// device pixels wide.
fn sharp_stripes(p: &Pixmap, y: u32, black_first: bool, each: u32) -> bool {
    (0..200).chain(4000..4200).all(|x| {
        let red = p.pixel(x, y).unwrap().red();
        let black = (x / each) % 2 == u32::from(!black_first);
        if black { red < 8 } else { red > 247 }
    })
}

#[test]
fn zoomed_out_the_canvas_draws_both_images_from_copies_that_stay_decoded() {
    let (d, page, keys) = doc("zoomed-out");
    // At 25% each shows 1500 px long: drawn from 4096 px copies, not decoded in full.
    render_view(&d, page, &view(0.25), &canvas()).unwrap();
    let first = keys.each_ref().map(|k| shown(&d, k, 1500.0));
    for (k, p) in keys.iter().zip(&first) {
        assert_eq!((p.width(), p.height()), (DISPLAY_SIDES[0], 5), "{k}");
        assert_eq!(images::cached_sides(k), vec![DISPLAY_SIDES[0]], "{k}: only its copy is decoded");
    }
    // Drawn again (a drag, a pan), both are still the copies decoded the first time.
    render_view(&d, page, &view(0.25), &canvas()).unwrap();
    for (k, p) in keys.iter().zip(&first) {
        assert!(Arc::ptr_eq(p, &shown(&d, k, 1500.0)), "{k} was decoded again");
    }
}

#[test]
fn exports_and_close_ups_draw_from_the_image_itself() {
    let (d, page, keys) = doc("close-up");
    // An export at 1x, and the canvas at 100% and 200%, show every pixel column.
    let export = render_view(&d, page, &view(1.0), &Options::default()).unwrap();
    assert!(sharp_stripes(&export, 4, true, 1) && sharp_stripes(&export, 2 * H + 4, false, 1));
    let at_100 = render_view(&d, page, &view(1.0), &canvas()).unwrap();
    assert!(sharp_stripes(&at_100, 4, true, 1) && sharp_stripes(&at_100, 2 * H + 4, false, 1));
    let at_200 = render_view(&d, page, &view(2.0), &canvas()).unwrap();
    assert!(sharp_stripes(&at_200, 8, true, 2));
    assert_eq!(images::cached_sides(&keys[0]), vec![16_384], "the image itself, as exports draw it");
}

#[test]
fn the_copy_is_the_smallest_that_holds_what_shows() {
    let side = |long, shown| images::display_side(long, shown);
    assert_eq!(side(6000, 1500.0), 4096);
    assert_eq!(side(16_000, 4096.0), 4096);
    assert_eq!(side(16_000, 4097.0), 8192);
    // Shown at more than the largest copy, or an image no larger than the copy: the image itself.
    assert_eq!(side(16_000, 9000.0), 16_384);
    assert_eq!(side(6000, 5000.0), 16_384);
    assert_eq!(side(4096, 100.0), 16_384);
    // An unknown size or a meaningless scale: the image itself.
    assert_eq!(side(0, 100.0), 16_384);
    assert_eq!(side(16_000, f64::NAN), 16_384);
}

#[test]
fn two_16k_images_fit_the_cache_as_copies_but_not_in_full() {
    // 16 000 × 10 000 (#1483's size class): 640 MB each in full, 42 MB at 4096 and 168 MB at 8192.
    let bytes = |w: u64, h: u64| w * h * 4;
    assert!(2 * bytes(16_000, 10_000) > MAX_CACHED_BYTES);
    assert!(2 * bytes(8192, 5120) <= MAX_CACHED_BYTES);
    assert!(2 * bytes(4096, 2560) <= MAX_CACHED_BYTES);
}
