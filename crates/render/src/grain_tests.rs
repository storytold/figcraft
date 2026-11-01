//! Noise and texture: grains in the layer's own coordinates (stable under pans), colour modes,
//! size and density; texture scattering edges, clipped or not.

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Effect, EffectKind, NodeId, NodeKind, Noise, NoiseMode, Texture};

use super::*;

/// A mid-grey 100 × 100 rectangle at (50, 50) with `kind`.
fn square(kind: EffectKind) -> (Document, NodeId) {
    let mut d = Document::new("grain");
    let p = d.first_page().unwrap();
    let r = d.create(p, NodeKind::Rectangle, "R", 50.0, 50.0, 100.0, 100.0).unwrap();
    let n = d.get_mut(r).unwrap();
    n.fills = vec![Paint::solid(Color::from_hex("808080").unwrap())];
    n.effects = vec![Effect { kind, visible: true }];
    (d, r)
}

/// Node `id` on a transparent `w` × `h` view whose top-left is page `origin`.
fn render_at(d: &Document, id: NodeId, origin: (f64, f64), scale: f64) -> Pixmap {
    let mut pix = Pixmap::new(200, 200).unwrap();
    render_nodes_into(d, &[id], &View { origin, scale, width: 200, height: 200 }, &mut pix);
    pix
}

fn px(p: &Pixmap, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width() + x) * 4) as usize;
    [p.data()[i], p.data()[i + 1], p.data()[i + 2], p.data()[i + 3]]
}

/// The distinct pixels of the square's inside (60..140).
fn inside(p: &Pixmap) -> Vec<[u8; 4]> {
    (60..140).flat_map(|y| (60..140).map(move |x| (x, y))).map(|(x, y)| px(p, x, y)).collect()
}

fn noise(n: Noise) -> EffectKind {
    EffectKind::Noise(n)
}

#[test]
fn noise_density_sets_the_share_of_grains() {
    let grey = [128, 128, 128, 255];
    for (density, lo, hi) in [(0.0, 0.0, 0.0), (0.3, 0.25, 0.35), (1.0, 1.0, 1.0)] {
        let (d, r) = square(noise(Noise { density, color: Color::BLACK, size: 0.5, ..Noise::default() }));
        let p = render_at(&d, r, (0.0, 0.0), 1.0);
        let px = inside(&p);
        let share = px.iter().filter(|c| **c != grey).count() as f64 / px.len() as f64;
        assert!((lo..=hi).contains(&share), "density {density}: {share}");
    }
}

#[test]
fn noise_colour_modes() {
    let (red, blue) = (Color::from_hex("FF0000").unwrap(), Color::from_hex("0000FF").unwrap());
    // Mono: grains of the one colour.
    let (d, r) = square(noise(Noise { density: 1.0, color: red, ..Noise::default() }));
    assert!(inside(&render_at(&d, r, (0.0, 0.0), 1.0)).iter().all(|c| *c == [255, 0, 0, 255]));
    // Duo: grains of both colours, about half each.
    let (d, r) = square(noise(Noise { mode: NoiseMode::Duo, density: 1.0, color: red, secondary: blue, ..Noise::default() }));
    let px = inside(&render_at(&d, r, (0.0, 0.0), 1.0));
    let reds = px.iter().filter(|c| **c == [255, 0, 0, 255]).count();
    let blues = px.iter().filter(|c| **c == [0, 0, 255, 255]).count();
    assert_eq!(reds + blues, px.len());
    assert!(reds > px.len() / 3 && blues > px.len() / 3, "{reds} / {blues}");
    // Multi: grains of many colours at the noise's opacity.
    let (d, r) = square(noise(Noise { mode: NoiseMode::Multi, density: 1.0, opacity: 1.0, ..Noise::default() }));
    let mut colours = inside(&render_at(&d, r, (0.0, 0.0), 1.0));
    colours.sort_unstable();
    colours.dedup();
    assert!(colours.len() > 1000, "{}", colours.len());
}

#[test]
fn noise_grains_have_their_size_and_stay_put() {
    // Size 2: grains 4 units across, aligned to the layer (at 50, 50: grains start at 62).
    let (d, r) = square(noise(Noise { density: 0.5, size: 2.0, color: Color::BLACK, ..Noise::default() }));
    let a = render_at(&d, r, (0.0, 0.0), 1.0);
    for gy in 0..20 {
        for gx in 0..20 {
            let (x0, y0) = (62 + gx * 4, 62 + gy * 4);
            let c = px(&a, x0, y0);
            for (x, y) in [(x0 + 3, y0), (x0, y0 + 3), (x0 + 3, y0 + 3)] {
                assert_eq!(px(&a, x, y), c, "grain at ({x0}, {y0})");
            }
        }
    }
    // Panning by whole pixels moves the grains with the layer.
    let b = render_at(&d, r, (7.0, 3.0), 1.0);
    for (x, y) in [(60, 60), (90, 75), (130, 131)] {
        assert_eq!(px(&a, x, y), px(&b, x - 7, y - 3));
    }
    // Zooming scales them: at 2× a grain is 8 pixels.
    let z = render_at(&d, r, (50.0, 50.0), 2.0);
    let c = px(&z, 16, 16);
    assert_eq!(px(&z, 23, 23), c);
}

/// Panning by whole pixels at a zoom where grain edges fall on pixel centres (1.7: a 1-unit grain
/// is 1.7 pixels, every tenth edge on a centre) reaches those edges from another origin, with
/// another rounding error: the grains still stay put.
#[test]
fn grains_stay_put_where_their_edges_fall_on_pixel_centres() {
    let s = 1.7;
    for (why, kind) in [
        ("noise", noise(Noise { density: 0.5, size: 0.5, color: Color::BLACK, ..Noise::default() })),
        ("texture", EffectKind::Texture(Texture { size: 0.5, radius: 12.0, clip_to_shape: false, ..Texture::default() })),
    ] {
        let (d, r) = square(kind);
        let a = render_at(&d, r, (0.0, 0.0), s);
        for (dx, dy) in [(79u32, 62u32), (13, 7), (31, 47), (5, 99)] {
            let b = render_at(&d, r, (f64::from(dx) / s, f64::from(dy) / s), s);
            let off = (0..200 - dy).flat_map(|y| (0..200 - dx).map(move |x| (x, y))).filter(|&(x, y)| px(&a, x + dx, y + dy) != px(&b, x, y)).count();
            assert_eq!(off, 0, "{why}, a pan by ({dx}, {dy}) pixels");
        }
    }
}

#[test]
fn noise_blend_mode_applies() {
    let (d, r) = square(noise(Noise { density: 1.0, color: Color::WHITE, blend: figcraft_color::BlendMode::Multiply, ..Noise::default() }));
    // White multiplied changes nothing.
    assert!(inside(&render_at(&d, r, (0.0, 0.0), 1.0)).iter().all(|c| *c == [128, 128, 128, 255]));
}

#[test]
fn texture_roughens_edges() {
    let plain = {
        let (d, r) = square(EffectKind::Texture(Texture { radius: 0.0, ..Texture::default() }));
        render_at(&d, r, (0.0, 0.0), 1.0)
    };
    // Radius 6: pixels scatter up to 6 past the edge; the inside of a flat fill stays flat.
    let (d, r) = square(EffectKind::Texture(Texture { radius: 6.0, size: 0.5, ..Texture::default() }));
    let rough = render_at(&d, r, (0.0, 0.0), 1.0);
    let outside: usize = (40..50).flat_map(|y| (60..140).map(move |x| (x, y))).filter(|&(x, y)| px(&rough, x, y)[3] > 0).count();
    assert!(outside > 50, "{outside}");
    assert_eq!(px(&rough, 100, 100), px(&plain, 100, 100));
    let edge_holes: usize = (50..56).flat_map(|y| (60..140).map(move |x| (x, y))).filter(|&(x, y)| px(&rough, x, y)[3] == 0).count();
    assert!(edge_holes > 20, "{edge_holes}");
    // Clipped to the shape: nothing outside.
    let (d, r) = square(EffectKind::Texture(Texture { radius: 6.0, clip_to_shape: true, ..Texture::default() }));
    let clipped = render_at(&d, r, (0.0, 0.0), 1.0);
    assert!((0..200).all(|x| (0..48).all(|y| px(&clipped, x, y)[3] == 0)));
}

#[test]
fn hostile_grain_values_do_not_crash() {
    for n in [
        Noise { size: f64::NAN, density: f64::INFINITY, opacity: f32::NAN, ..Noise::default() },
        Noise { size: 0.0, size_y: Some(-5.0), density: -1.0, ..Noise::default() },
        Noise { size: 1e300, mode: NoiseMode::Multi, ..Noise::default() },
    ] {
        let (d, r) = square(noise(n));
        let _ = render_at(&d, r, (0.0, 0.0), 1.0);
    }
    for t in [Texture { radius: f64::NAN, ..Texture::default() }, Texture { radius: 1e12, size: 0.0, ..Texture::default() }] {
        let (d, r) = square(EffectKind::Texture(t));
        let _ = render_at(&d, r, (0.0, 0.0), 1.0);
    }
}
