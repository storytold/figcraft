//! A filtered backdrop replaces the sampled pixels, including their transparency.

use figcraft_color::{Color, Paint};
use figcraft_doc::{Blur, Document, Effect, EffectKind, Glass, NodeId, NodeKind};
use figcraft_geom::Shape;

use super::*;

fn scene(kind: EffectKind) -> (Document, NodeId, NodeId) {
    let mut d = Document::new("transparent backdrop");
    let page = d.first_page().unwrap();
    let frame = d.create(page, NodeKind::Frame, "Frame", 0.0, 0.0, 160.0, 120.0).unwrap();
    d.get_mut(frame).unwrap().fills.clear();
    let stripe = d.create(frame, NodeKind::Rectangle, "Translucent stripe", 0.0, 0.0, 80.0, 120.0).unwrap();
    d.get_mut(stripe).unwrap().fills = vec![Paint::solid(Color::new(0.8, 0.2, 0.1, 0.5))];
    let pane = d.create(frame, NodeKind::Rectangle, "Pane", 40.0, 30.0, 80.0, 60.0).unwrap();
    let n = d.get_mut(pane).unwrap();
    n.fills.clear();
    n.effects = vec![Effect { kind, visible: true }];
    (d, frame, pane)
}

fn without(d: &Document, frame: NodeId, pane: NodeId) -> Pixmap {
    let mut plain = d.clone();
    plain.get_mut(pane).unwrap().visible = false;
    render_node(&plain, frame, 1.0).unwrap()
}

fn pixel(p: &Pixmap, x: usize, y: usize) -> &[u8] {
    let i = (y * p.width() as usize + x) * 4;
    &p.data()[i..i + 4]
}

#[test]
fn neutral_glass_preserves_transparent_backdrop_pixels() {
    let glass = Glass { frost: 0.0, refraction: 0.0, dispersion: 0.0, light_intensity: 0.0, ..Glass::default() };
    let (d, frame, pane) = scene(EffectKind::Glass(glass));
    let got = render_node(&d, frame, 1.0).unwrap();
    let want = without(&d, frame, pane);
    assert_eq!(got.data(), want.data(), "neutral glass must not paint a second copy of the translucent stripe");
}

#[test]
fn neutral_glass_preserves_pixels_at_antialiased_edges() {
    let glass = Glass { frost: 0.0, refraction: 0.0, dispersion: 0.0, light_intensity: 0.0, ..Glass::default() };
    let (mut d, frame, pane) = scene(EffectKind::Glass(glass));
    let n = d.get_mut(pane).unwrap();
    n.transform = Affine::translate((40.25, 30.25));
    n.corners.radii = [14.0; 4];
    n.opacity = 0.5;
    let got = render_node(&d, frame, 1.0).unwrap();
    let want = without(&d, frame, pane);
    let worst = got.data().iter().zip(want.data()).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
    assert!(worst <= 1, "neutral antialiased glass changed a channel by {worst}");
}

#[test]
fn background_blur_replaces_sharp_translucent_pixels() {
    let (d, frame, pane) = scene(EffectKind::BackgroundBlur(Blur::uniform(12.0)));
    let got = render_node(&d, frame, 1.0).unwrap();
    let mut want = without(&d, frame, pane);
    fx::blur(&mut want, 6.0);
    for y in 40..80 {
        for x in 60..100 {
            let (a, b) = (pixel(&got, x, y), pixel(&want, x, y));
            assert!(a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 2), "at ({x}, {y}): {a:?} vs {b:?}");
        }
    }
}

#[test]
fn backdrop_opacity_interpolates_filtered_and_original_pixels() {
    let (mut d, frame, pane) = scene(EffectKind::BackgroundBlur(Blur::uniform(12.0)));
    d.get_mut(pane).unwrap().opacity = 0.5;
    let got = render_node(&d, frame, 1.0).unwrap();
    let original = without(&d, frame, pane);
    let mut blurred = original.clone();
    fx::blur(&mut blurred, 6.0);
    for y in 40..80 {
        for x in 60..100 {
            for ((a, b), c) in pixel(&got, x, y).iter().zip(pixel(&original, x, y)).zip(pixel(&blurred, x, y)) {
                let want = (u16::from(*b) + u16::from(*c)).div_ceil(2) as u8;
                assert!(a.abs_diff(want) <= 2, "at ({x}, {y}): {a} vs {want}");
            }
        }
    }
}

#[test]
fn translucent_fills_and_strokes_paint_over_replaced_backdrop() {
    let (mut d, frame, pane) = scene(EffectKind::BackgroundBlur(Blur::uniform(12.0)));
    let n = d.get_mut(pane).unwrap();
    n.fills = vec![Paint::solid(Color::new(0.1, 0.3, 0.9, 0.25))];
    n.strokes = vec![Paint::solid(Color::new(0.1, 0.9, 0.2, 0.6))];
    n.stroke.weight = 10.0;
    let got = render_node(&d, frame, 1.0).unwrap();
    let mut want = without(&d, frame, pane);
    fx::blur(&mut want, 6.0);
    let mut plain = d.clone();
    plain.get_mut(pane).unwrap().effects.clear();
    render_nodes_into(&plain, &[pane], &View { origin: (0.0, 0.0), scale: 1.0, width: 160, height: 120 }, &mut want);
    for y in 35..85 {
        for x in 45..115 {
            let (a, b) = (pixel(&got, x, y), pixel(&want, x, y));
            assert!(a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 2), "at ({x}, {y}): {a:?} vs {b:?}");
        }
    }
}

#[test]
fn fractional_replacement_coverage_preserves_content_opacity() {
    for fraction in [0.125, 0.25, 0.5, 0.75, 0.875] {
        let shape = (figcraft_geom::Rect::new(0.0, 0.0, fraction, 1.0).to_path(0.1), FillRule::Winding);
        let geometric = shadow::spread_alpha(std::slice::from_ref(&shape), 0.0, Affine::IDENTITY, 1, 1).unwrap();
        let c = f32::from(geometric.data()[3]) / 255.0;
        for alpha in [0, 1, 64, 128, 254, 255] {
            let mut original = Pixmap::new(1, 1).unwrap();
            original.fill(tiny_skia::Color::from_rgba8(255, 0, 0, alpha));
            let mut back = original.clone();
            fx::mask_by(&mut back, &geometric, false);
            for content_alpha in [0, 1, 64, 128, 254, 255] {
                let mut content = Pixmap::new(1, 1).unwrap();
                content.fill(tiny_skia::Color::from_rgba8(0, 0, 255, content_alpha));
                let mut layer = back.clone();
                paint::composite(&mut layer, (0, 0), &content, 1.0, figcraft_color::BlendMode::Normal);
                for opacity in [0.25, 0.5, 1.0] {
                    let mut got = original.clone();
                    if let Some(coverage) = backdrop::replacement_coverage(&shape, &back, &layer, Affine::IDENTITY) {
                        backdrop::composite_replacing(&mut got, (0, 0), &layer, &coverage, opacity);
                    } else {
                        paint::composite(&mut got, (0, 0), &layer, opacity, figcraft_color::BlendMode::Normal);
                    }
                    let coverage = c + (1.0 - c) * f32::from(content_alpha) / 255.0;
                    for ((actual, source), destination) in got.data().iter().zip(layer.data()).zip(original.data()) {
                        let expected = (f32::from(*source) * opacity + f32::from(*destination) * (1.0 - opacity * coverage)).round() as u8;
                        assert!(
                            actual.abs_diff(expected) <= 1,
                            "backdrop {alpha}, content {content_alpha}, opacity {opacity}: {actual} vs {expected}"
                        );
                    }
                }
            }
        }
    }
}
