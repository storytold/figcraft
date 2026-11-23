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
/// `(x, 0)` in a white 120 × 100 frame; returns the frame.
fn line_at(x: f64) -> (Document, figcraft_doc::NodeId) {
    let mut img = Pixmap::new(400, 400).unwrap();
    for (i, c) in img.data_mut().chunks_exact_mut(4).enumerate() {
        let v = if i % 400 == 201 { 0 } else { 255 };
        c.copy_from_slice(&[v, v, v, 255]);
    }
    let (mut d, p) = page();
    d.images.insert("line".into(), ImageData { bytes: Arc::new(encode_png(&img).unwrap()), width: 400, height: 400 });
    let f = d.create(p, NodeKind::Frame, "Frame", 0.0, 0.0, 120.0, 100.0).unwrap();
    d.get_mut(f).unwrap().fills = vec![Paint::solid(Color::WHITE)];
    let r = d.create(f, NodeKind::Rectangle, "Image", x, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::image("line", ImageScaleMode::Fill)];
    (d, f)
}

#[test]
fn an_image_shown_smaller_exports_the_average_of_what_each_pixel_covers() {
    // Shown at a quarter of its size, the line is a quarter of a pixel wide, at x + 50.25. The
    // pixels it crosses share its quarter pixel of black by how much of it each covers (sampled at
    // pixel centres instead, it vanishes or comes out a whole pixel dark).
    for at in [0.0, 10.5, 7.3, 3.9, 0.6] {
        let (d, f) = line_at(at);
        let pix = render_node(&d, f, 1.0).unwrap();
        let w = pix.width() as usize;
        let ink = |x: usize| 255.0 - f64::from(pix.data()[(50 * w + x) * 4]);
        let (l, r) = (at + 50.25, at + 50.5);
        for x in 0..w {
            let covered = (r.min(x as f64 + 1.0) - l.max(x as f64)).max(0.0);
            assert!((ink(x) - 255.0 * covered).abs() < 3.0, "layer at x {at}: pixel {x} holds {} of black", ink(x));
        }
    }
}

/// Where a shape's edge cuts a pixel, an image fitted to that edge fades it once (by the shape's
/// coverage), not again for the image area past the edge.
#[test]
fn a_fitted_image_shown_smaller_fades_the_shapes_edge_once() {
    let mut img = Pixmap::new(8, 8).unwrap();
    img.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
    let (mut d, p) = page();
    d.images.insert("black".into(), ImageData { bytes: Arc::new(encode_png(&img).unwrap()), width: 8, height: 8 });
    let f = d.create(p, NodeKind::Frame, "Frame", 0.0, 0.0, 8.0, 8.0).unwrap();
    d.get_mut(f).unwrap().fills = vec![Paint::solid(Color::WHITE)];
    // 4 wide, so the image shows at half its size, fitted edge to edge across.
    let r = d.create(f, NodeKind::Rectangle, "Image", 0.25, 0.0, 4.0, 6.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::image("black", ImageScaleMode::Fit)];
    let pix = render_node(&d, f, 1.0).unwrap();
    let w = pix.width() as usize;
    let ink = |x: usize| 255.0 - f64::from(pix.data()[(3 * w + x) * 4]);
    assert!((ink(0) - 255.0 * 0.75).abs() < 4.0, "the first pixel, three quarters covered, holds {}", ink(0));
    assert!(ink(1) > 250.0, "the next, inside, holds {}", ink(1));
}

/// Text painted only through its runs' fills draws something, so it counts for the export.
#[test]
fn text_painted_by_its_runs_counts_for_the_export() {
    let (mut d, p) = page();
    let mut td = figcraft_doc::TextData::new("Hi");
    td.runs = vec![figcraft_doc::StyleRun {
        start: 0,
        end: 2,
        style: figcraft_doc::TextStyle { fills: Some(vec![Paint::solid(Color::BLACK)]), ..td.style.clone() },
    }];
    let t = d.create(p, NodeKind::Text(td), "T", 0.0, 0.0, 40.0, 20.0).unwrap();
    let n = d.get_mut(t).unwrap();
    n.fills.clear();
    assert!(!n.draws_nothing());
    let NodeKind::Text(td) = &mut n.kind else { panic!("not text") };
    td.runs.clear();
    assert!(n.draws_nothing(), "without the runs' fills, nothing");
}
