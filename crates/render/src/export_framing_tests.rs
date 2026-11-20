//! How an export frames and samples a layer, as the reference tool's exports do (measured on its
//! renders of the same scenes): a drop shadow counts as far as its blur radius (its faint tail past
//! that stays out), a layer inside that draws nothing doesn't count, and an image shown smaller
//! than its pixels comes out as the average of the image under each pixel, wherever it falls
//! between pixels.

use std::sync::Arc;

use figcraft_color::{BlendMode, Color, ImageScaleMode, Paint};
use figcraft_doc::{Document, Effect, EffectKind, ImageData, NodeKind, Shadow};

use super::paint_behaviour_tests::page;
use crate::{Pixmap, encode_png, export_bounds, render_node};

fn shadow(radius: f64, offset: (f64, f64)) -> Effect {
    let color = Color { a: 0.3, ..Color::BLACK };
    Effect { kind: EffectKind::DropShadow(Shadow { color, offset, radius, blend: BlendMode::Normal, ..Shadow::default() }), visible: true }
}

#[test]
fn a_shadow_reaches_into_the_export_as_far_as_its_blur_radius() {
    let (mut d, p) = page();
    let b = d.create(p, NodeKind::Frame, "Button", 100.0, 100.0, 44.0, 44.0).unwrap();
    let n = d.get_mut(b).unwrap();
    (n.fills, n.effects) = (vec![Paint::solid(Color::WHITE)], vec![shadow(16.0, (0.0, 4.0))]);
    let r = export_bounds(&d, b).unwrap();
    // 44 + 2 × 16 square, moved down by the offset: 76 × 76, as the reference tool exports it.
    assert_eq!((r.x0, r.y0, r.x1, r.y1), (84.0, 88.0, 160.0, 164.0));
}

#[test]
fn a_layer_that_draws_nothing_stays_out_of_the_export() {
    let (mut d, p) = page();
    let f = d.create(p, NodeKind::Frame, "Illustration", 0.0, 0.0, 300.0, 225.0).unwrap();
    let n = d.get_mut(f).unwrap();
    (n.fills, n.clip) = (vec![Paint::solid(Color::WHITE)], false);
    // A placeholder far larger than the frame, its image taken out (no fills).
    let ph = d.create(f, NodeKind::Rectangle, "image 15", -445.0, -438.0, 1448.0, 1086.0).unwrap();
    d.get_mut(ph).unwrap().fills.clear();
    let r = export_bounds(&d, f).unwrap();
    assert_eq!((r.x0, r.y0, r.x1, r.y1), (0.0, 0.0, 300.0, 225.0));
    // Given a fill, it counts.
    d.get_mut(ph).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    assert!(export_bounds(&d, f).unwrap().width() > 1000.0);
}

/// A 400 × 400 white image with one black column of pixels (x = 201), on a 100 × 100 layer at
/// `(x, 0)`.
fn line_at(x: f64) -> (Document, figcraft_doc::NodeId) {
    let mut img = Pixmap::new(400, 400).unwrap();
    for (i, c) in img.data_mut().chunks_exact_mut(4).enumerate() {
        let v = if i % 400 == 201 { 0 } else { 255 };
        c.copy_from_slice(&[v, v, v, 255]);
    }
    let (mut d, p) = page();
    d.images.insert("line".into(), ImageData { bytes: Arc::new(encode_png(&img).unwrap()), width: 400, height: 400 });
    let r = d.create(p, NodeKind::Rectangle, "Image", x, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::image("line", ImageScaleMode::Fill)];
    (d, r)
}

#[test]
fn an_image_shown_smaller_exports_the_average_of_what_each_pixel_covers() {
    // Shown at a quarter of its size, the line is a quarter of a pixel wide: wherever it falls,
    // the pixels it crosses hold a quarter pixel's worth of black between them (sampled at pixel
    // centres instead, it vanishes or comes out a whole pixel dark).
    for at in [0.0, 10.5, 7.3, 3.9] {
        let (d, r) = line_at(at);
        let pix = render_node(&d, r, 1.0).unwrap();
        let w = pix.width() as usize;
        let y = 50;
        let ink: f64 = (0..w).map(|x| 255.0 - f64::from(pix.data()[(y * w + x) * 4])).sum();
        assert!((ink - 255.0 / 4.0).abs() < 6.0, "layer at x {at}: {ink} of black in the row");
    }
}
