//! Rendering in parallel bands matches rendering in one piece: effects that look past a band's
//! rows (inner shadows, background blurs) must not draw the band's edges into the picture.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use super::*;
use figcraft_color::{ImageScaleMode, Paint};
use figcraft_doc::{Effect, EffectKind, ImageData, Shadow};

fn inner_shadow(offset: (f64, f64), radius: f64, spread: f64) -> Effect {
    Effect {
        kind: EffectKind::InnerShadow(Shadow { color: Color::from_hex("000000B3").unwrap(), offset, radius, spread, ..Default::default() }),
        visible: true,
    }
}

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(64, 64, |x, y| image::Rgba([(x * 4) as u8, (y * 4) as u8, 160, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// An image-filled ellipse and a rotated solid rectangle, with inner shadows (`shadows`), and a
/// glass panel (background blur) over both (`glass`).
fn scene(shadows: bool, glass: bool) -> (Document, NodeId) {
    let mut d = Document::new("bands");
    let p = d.first_page().unwrap();
    // Image ids are content hashes in real documents and key the shared decode cache, so test
    // fixtures running in parallel need ids of their own.
    d.images.insert("band-tests-gradient".into(), ImageData { asset: None, bytes: Arc::new(png()), width: 64, height: 64 });
    let e = d.create(p, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "E", 0.0, 0.0, 200.0, 200.0).unwrap();
    let n = d.get_mut(e).unwrap();
    n.fills = vec![Paint::image("band-tests-gradient", ImageScaleMode::Fill)];
    if shadows {
        n.effects = vec![inner_shadow((0.0, 4.0), 8.0, 0.0)];
    }
    let r = d.create(p, NodeKind::Rectangle, "R", 0.0, 0.0, 150.0, 180.0).unwrap();
    let n = d.get_mut(r).unwrap();
    n.fills = vec![Paint::solid(Color::from_hex("3C8CE6").unwrap())];
    n.transform = Affine::translate((260.0, 10.0)) * Affine::rotate(0.35);
    if shadows {
        n.effects = vec![inner_shadow((4.0, 4.0), 12.0, 2.0)];
    }
    if glass {
        let g = d.create(p, NodeKind::Rectangle, "Glass", 40.0, 60.0, 320.0, 120.0).unwrap();
        let n = d.get_mut(g).unwrap();
        n.fills = vec![Paint::solid(Color::from_hex("FFFFFF33").unwrap())];
        n.effects = vec![Effect { kind: EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(12.0)), visible: true }];
    }
    (d, p)
}

fn view(scale: f64) -> View {
    View { origin: (-10.0, -10.0), scale, width: (440.0 * scale) as u32, height: (240.0 * scale) as u32 }
}

/// Per row: the mean channel difference and how many pixels differ by more than 2. A band artifact
/// differs along a whole row; edge antialiasing may differ in a few pixels per edge (a curve
/// clipped at a band's edge rasterises a hair differently, most where it runs nearly level).
fn row_diffs(a: &Pixmap, b: &Pixmap) -> Vec<(f64, usize)> {
    let row = a.width() as usize * 4;
    a.data()
        .chunks(row)
        .zip(b.data().chunks(row))
        .map(|(x, y)| {
            let sum: f64 = x.iter().zip(y).map(|(p, q)| f64::from(p.abs_diff(*q))).sum();
            let off = x.chunks(4).zip(y.chunks(4)).filter(|(p, q)| p.iter().zip(*q).any(|(c, d)| c.abs_diff(*d) > 2)).count();
            (sum / row as f64, off)
        })
        .collect()
}

#[test]
fn bands_match_one_piece_rendering() {
    let (d, p) = scene(true, true);
    for scale in [1.0, 2.0] {
        let v = view(scale);
        let one = render_view_single(&d, p, &v, &Options::default()).unwrap();
        for bands in [2, 3, 7] {
            let banded = render_bands(&d, p, &v, &Options::default(), bands).unwrap();
            assert_eq!((banded.width(), banded.height()), (one.width(), one.height()));
            for (y, (mean, off)) in row_diffs(&one, &banded).into_iter().enumerate() {
                assert!(
                    mean < 0.25 && off * 20 <= v.width as usize,
                    "scale {scale}, {bands} bands: row {y} differs ({off} pixels, {mean:.2} on average)"
                );
            }
        }
    }
}

#[test]
fn inner_shadow_leaves_the_interior_alone() {
    // Far from the ellipse's edge, the image fill shows exactly as without the shadow, on every row
    // (the shadow used to draw a band at each render band's edge).
    let (with, p) = scene(true, false);
    let (without, _) = scene(false, false);
    let v = view(2.0);
    let a = render_bands(&with, p, &v, &Options::default(), 6).unwrap();
    let b = render_bands(&without, p, &v, &Options::default(), 6).unwrap();
    // Ellipse centre (100, 100) page units; the shadow reaches 3 sigma + offset = 16 units in.
    let (cx, cy, r) = (110.0 * 2.0, 110.0 * 2.0, (100.0 - 20.0) * 2.0);
    let mut checked = 0;
    for y in 0..v.height {
        for x in 0..v.width {
            let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
            if dx * dx + dy * dy > r * r {
                continue;
            }
            let i = ((y * v.width + x) * 4) as usize;
            assert_eq!(a.data()[i..i + 4], b.data()[i..i + 4], "pixel {x},{y}");
            checked += 1;
        }
    }
    assert!(checked > 10_000, "{checked}");
}

#[test]
fn huge_backdrop_blur_renders_in_one_piece() {
    let (mut d, p) = scene(true, true);
    let g = d.find_by_name("Glass", None)[0];
    d.get_mut(g).unwrap().effects = vec![Effect { kind: EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(1e6)), visible: true }];
    assert!(render_bands(&d, p, &view(1.0), &Options::default(), 4).is_none());
    assert!(render_view(&d, p, &view(1.0), &Options::default()).is_some());
}
