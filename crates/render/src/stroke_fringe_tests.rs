//! An opaque stroke meets what lies on each side of it with nothing else showing between: no
//! light fill fringe along an inside stroke's outer edge, no background gap along an outside
//! stroke's inner edge (#2635).

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, NodeKind, StrokeAlign};

use super::*;

const DARK: Color = Color { r: 30.0 / 255.0, g: 27.0 / 255.0, b: 75.0 / 255.0, a: 1.0 };

/// The red channel along the middle row of a 40-unit `kind` at (`at`, `at`) with a black 4-unit
/// stroke, its fill `fill`, over `background`.
fn middle_row(kind: &NodeKind, smoothing: f64, align: StrokeAlign, at: f64, fill: Color, background: Color) -> Vec<u8> {
    let mut d = Document::new("fringe");
    let p = d.first_page().unwrap();
    let n = d.create(p, kind.clone(), "S", at, at, 40.0, 40.0).unwrap();
    let node = d.get_mut(n).unwrap();
    node.fills = vec![Paint::solid(fill)];
    node.strokes = vec![Paint::solid(Color::BLACK)];
    (node.stroke.weight, node.stroke.align, node.corners.smoothing) = (4.0, align, smoothing);
    if smoothing > 0.0 {
        node.corners.radii = [8.0; 4];
    }
    let mut pix = Pixmap::new(80, 80).unwrap();
    let c = |v: f32| (v * 255.0).round() as u8;
    pix.fill(tiny_skia::Color::from_rgba8(c(background.r), c(background.g), c(background.b), 255));
    render_nodes_into(&d, &[n], &View { origin: (0.0, 0.0), scale: 1.0, width: 80, height: 80 }, &mut pix);
    let y = (at + 20.0) as u32;
    (0..80).map(|x| pix.pixel(x, y).unwrap().red()).collect()
}

#[test]
fn an_opaque_stroke_shows_nothing_lighter_than_its_dark_neighbour_at_either_edge() {
    let dark = (DARK.r * 255.0).round() as u8;
    let ellipse = NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 };
    // Smoothed corners take the general outline rather than the rect ring.
    let shapes = [(&NodeKind::Frame, 0.0), (&NodeKind::Rectangle, 0.0), (&ellipse, 0.0), (&NodeKind::Rectangle, 0.6)];
    for (kind, smoothing) in shapes {
        for align in [StrokeAlign::Inside, StrokeAlign::Center, StrokeAlign::Outside] {
            for at in [20.4, 20.7] {
                let middle = at
                    + match align {
                        StrokeAlign::Inside => 2.0,
                        StrokeAlign::Center => 0.0,
                        StrokeAlign::Outside => -2.0,
                    };
                let centre = (at + 20.0) as usize;
                // Outside the stroke's middle on a dark page under a white fill, then inside it
                // under a dark fill on a white page: no pixel lighter than the dark side.
                let outer = middle_row(kind, smoothing, align, at, Color::WHITE, DARK);
                let outer = &outer[..middle.floor() as usize];
                let inner = middle_row(kind, smoothing, align, at, DARK, Color::WHITE);
                let inner = &inner[middle.ceil() as usize..centre];
                assert!(outer.iter().all(|&r| r <= dark), "{kind:?} smoothing {smoothing}, {align:?} at {at}: outer edge {outer:?}");
                assert!(inner.iter().all(|&r| r <= dark), "{kind:?} smoothing {smoothing}, {align:?} at {at}: inner edge {inner:?}");
            }
        }
    }
}
