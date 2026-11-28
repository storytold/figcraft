//! Downscaled images (#2664): an image shown at half its pixels or fewer is drawn from a copy
//! averaged down to under twice that size, so one-pixel detail blends into an even grey instead of
//! aliasing into moiré bands, on the canvas and in exports alike.

use std::sync::Arc;

use figcraft_color::{ImageScaleMode, Paint, PaintKind};
use figcraft_doc::{Document, ImageData, NodeId, NodeKind};
use figcraft_geom::Affine;

use crate::{Options, Pixmap, View, encode_png, render_view};

/// The image's side: smaller than every canvas copy, so the canvas draws it from itself too.
const SIDE: u32 = 200;

/// A `SIDE` × `SIDE` checkerboard of one-pixel black and white squares filling a box its size: the
/// document, its page and the box.
fn checkerboard_doc() -> (Document, NodeId, NodeId) {
    let mut img = Pixmap::new(SIDE, SIDE).unwrap();
    for (i, c) in (0u32..).zip(img.data_mut().chunks_exact_mut(4)) {
        let black = (i % SIDE + i / SIDE).is_multiple_of(2);
        c.copy_from_slice(if black { &[0, 0, 0, 255] } else { &[255, 255, 255, 255] });
    }
    let mut d = Document::new("downscaled");
    let page = d.first_page().unwrap();
    d.images.insert("fine".into(), ImageData::new(Arc::new(encode_png(&img).unwrap()), SIDE, SIDE));
    let r = d.create(page, NodeKind::Rectangle, "Image", 0.0, 0.0, f64::from(SIDE), f64::from(SIDE)).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::image("fine", ImageScaleMode::Fill)];
    (d, page, r)
}

/// The darkest and lightest red over the image's pixels (but a 2-pixel rim, where it meets the
/// background) drawn at `scale` from `origin`.
fn red_range(d: &Document, page: NodeId, scale: f64, origin: (f64, f64), opts: &Options) -> (u8, u8) {
    let side = (f64::from(SIDE) * scale).floor() as u32;
    let pix = render_view(d, page, &View { origin, scale, width: side, height: side }, opts).unwrap();
    let inner = 2..side - 2;
    inner
        .clone()
        .flat_map(|y| inner.clone().map(move |x| (x, y)))
        .map(|(x, y)| pix.pixel(x, y).unwrap().red())
        .fold((255, 0), |(lo, hi), r| (lo.min(r), hi.max(r)))
}

#[test]
fn an_image_shown_at_half_its_size_or_less_blends_its_fine_detail_evenly() {
    let (d, page, _) = checkerboard_doc();
    let canvas = Options { lod: true, magnified_image_pixels: true, ..Default::default() };
    for (name, opts) in [("export", Options::default()), ("canvas", canvas)] {
        for scale in [0.5, 0.37, 0.23] {
            for origin in [(0.0, 0.0), (0.3, 0.7)] {
                let (lo, hi) = red_range(&d, page, scale, origin, &opts);
                assert!(lo >= 104 && hi <= 151, "{name} at {scale} from {origin:?}: reds {lo}..={hi}, not an even mid-grey");
            }
        }
    }
}

#[test]
fn an_adjusted_image_shown_smaller_averages_its_adjusted_pixels() {
    let (mut d, page, r) = checkerboard_doc();
    // Exposure brightens grey but leaves black black and white white.
    if let PaintKind::Image(img) = &mut d.get_mut(r).unwrap().fills[0].kind {
        img.filters.exposure = 1.0;
    }
    let canvas = Options { lod: true, magnified_image_pixels: true, ..Default::default() };
    for (name, opts) in [("export", Options::default()), ("canvas", canvas)] {
        let (lo, hi) = red_range(&d, page, 0.37, (0.3, 0.7), &opts);
        assert!(lo >= 104 && hi <= 151, "{name}: reds {lo}..={hi}, not the even mid-grey of black and white");
    }
}

#[test]
fn a_crop_stretching_one_axis_keeps_the_pixels_that_axis_shows() {
    // The whole image in a 400 × 2 box: each column two device pixels wide, the rows squeezed 100
    // to 1, so the view's row (at row 50's centre) shows every column black or white in turn.
    let (mut d, page, r) = checkerboard_doc();
    let node = d.get_mut(r).unwrap();
    node.width = f64::from(2 * SIDE);
    node.height = 2.0;
    if let PaintKind::Image(img) = &mut node.fills[0].kind {
        img.scale_mode = ImageScaleMode::Crop;
        img.crop = Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }
    let pix = render_view(&d, page, &View { origin: (0.0, 0.005), scale: 1.0, width: 2 * SIDE, height: 1 }, &Options::default()).unwrap();
    let reds: Vec<u8> = (0..2 * SIDE).map(|x| pix.pixel(x, 0).unwrap().red()).collect();
    let (lo, hi) = reds[4..reds.len() - 4].iter().fold((255, 0), |(lo, hi), &r| (lo.min(r), hi.max(r)));
    assert!(hi - lo > 64, "columns {lo}..={hi}: blended away, though each shows two device pixels wide");
}

/// A one-image document: `img` (`w` × `h`, under `key`: the cache is shared by every test) in a box
/// its size, scaled as `mode`.
fn image_doc(key: &str, img: &Pixmap, mode: ImageScaleMode) -> (Document, NodeId) {
    let (w, h) = (img.width(), img.height());
    let mut d = Document::new("downscaled");
    let page = d.first_page().unwrap();
    d.images.insert(key.into(), ImageData::new(Arc::new(encode_png(img).unwrap()), w, h));
    let r = d.create(page, NodeKind::Rectangle, "Image", 0.0, 0.0, f64::from(w), f64::from(h)).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::image(key, mode)];
    (d, page)
}

#[test]
fn a_thin_image_shown_smaller_still_fills_its_box() {
    // 200 × 3 at 0.5x: its copy is 100 × 2, proportions a third off from the image's.
    let mut img = Pixmap::new(200, 3).unwrap();
    img.fill(tiny_skia::Color::BLACK);
    let (d, page) = image_doc("thin", &img, ImageScaleMode::Fit);
    let pix = render_view(&d, page, &View { origin: (0.0, 0.0), scale: 0.5, width: 100, height: 1 }, &Options::default()).unwrap();
    let reds: Vec<u8> = (0..100).map(|x| pix.pixel(x, 0).unwrap().red()).collect();
    assert!(reds[1..99].iter().all(|&r| r < 8), "the box's row isn't all image (black): {reds:?}");
}

#[test]
fn a_transparent_image_shown_smaller_keeps_its_colour() {
    // Opaque red columns between transparent ones: averaged, a half-transparent pure red (light red
    // over the page's white).
    let mut img = Pixmap::new(200, 8).unwrap();
    for (i, c) in (0u32..).zip(img.data_mut().chunks_exact_mut(4)) {
        c.copy_from_slice(if (i % 200).is_multiple_of(2) { &[255, 0, 0, 255] } else { &[0, 0, 0, 0] });
    }
    let (d, page) = image_doc("half-clear", &img, ImageScaleMode::Fill);
    let pix = render_view(&d, page, &View { origin: (0.0, 0.0), scale: 0.37, width: 74, height: 2 }, &Options::default()).unwrap();
    for x in 2..72 {
        let c = pix.pixel(x, 1).unwrap();
        assert!(c.red() >= 245 && (100..=155).contains(&c.green()) && c.green().abs_diff(c.blue()) <= 4, "x {x}: {c:?}, not half red over white");
    }
}

#[test]
fn a_layer_squeezed_on_one_axis_keeps_the_pixels_its_other_axis_shows() {
    // The checkerboard's layer squeezed 100 to 1 vertically: each column one device pixel wide, so
    // the view's row (at row 50's centre) shows every column black or white in turn.
    let (mut d, page, r) = checkerboard_doc();
    d.get_mut(r).unwrap().transform = Affine::scale_non_uniform(1.0, 0.01);
    let pix = render_view(&d, page, &View { origin: (0.0, 0.005), scale: 1.0, width: SIDE, height: 1 }, &Options::default()).unwrap();
    let reds: Vec<u8> = (0..SIDE).map(|x| pix.pixel(x, 0).unwrap().red()).collect();
    let (lo, hi) = reds[4..reds.len() - 4].iter().fold((255, 0), |(lo, hi), &r| (lo.min(r), hi.max(r)));
    assert!(hi - lo > 64, "columns {lo}..={hi}: blended away, though each shows a device pixel wide");
}
