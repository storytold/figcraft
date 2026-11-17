//! Pan render timings: a page of 60 × 60 cards, each with a drop shadow and an inner shadow, seen
//! at 100% through a 1600 × 1100 device-pixel view. Run in release:
//! `cargo test --release -p figcraft-render --lib pan_render_timings -- --ignored --nocapture`.
//!
//! "full" renders the whole view; "pan" rows go through `Retained` as the canvas does, each step
//! moving the view by `step` device pixels and redrawing only the strips it uncovers.

use std::sync::Arc;
use std::time::Instant;

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Effect, EffectKind, NodeId, NodeKind, Shadow};

use crate::{Options, Retained, View, render_view};

const STEPS: u32 = 12;

/// The cards: (document, page).
fn cards() -> (Arc<Document>, NodeId) {
    let mut d = Document::new("pan bench");
    let p = d.first_page().unwrap();
    for y in 0..60u32 {
        for x in 0..60u32 {
            let c = d.create(p, NodeKind::Rectangle, "Card", f64::from(x) * 140.0, f64::from(y) * 110.0, 120.0, 90.0).unwrap();
            let n = d.get_mut(c).unwrap();
            n.fills = vec![Paint::solid(Color::new(x as f32 / 60.0, y as f32 / 60.0, 0.7, 1.0))];
            n.corners.radii = [12.0; 4];
            n.effects = vec![
                Effect { kind: EffectKind::DropShadow(Shadow { radius: 18.0, offset: (0.0, 6.0), ..Shadow::default() }), visible: true },
                Effect { kind: EffectKind::InnerShadow(Shadow { radius: 8.0, ..Shadow::default() }), visible: true },
            ];
        }
    }
    (Arc::new(d), p)
}

fn view(origin: (f64, f64)) -> View {
    View { origin, scale: 1.0, width: 1600, height: 1100 }
}

#[test]
#[ignore = "timing; run in release with --ignored --nocapture"]
fn pan_render_timings() {
    let (d, p) = cards();
    let opts = Options { lod: true, ..Options::default() };
    let t0 = Instant::now();
    for _ in 0..4 {
        render_view(&d, p, &view((1000.0, 1000.0)), &opts).unwrap();
    }
    println!("full: {:.1} ms", t0.elapsed().as_secs_f64() * 1000.0 / 4.0);
    for (name, step) in [("pan right", (60.0, 0.0)), ("pan down", (0.0, 60.0)), ("pan diagonally", (60.0, 24.0)), ("pan right fast", (240.0, 0.0))] {
        let mut r = Retained::default();
        let mut at = (1000.0, 1000.0);
        r.render(&d, p, &view(at), &opts).unwrap();
        let t0 = Instant::now();
        for _ in 0..STEPS {
            at = (at.0 + step.0, at.1 + step.1);
            r.render(&d, p, &view(at), &opts).unwrap();
        }
        println!("{name} ({:?} px a step): {:.1} ms a step", step, t0.elapsed().as_secs_f64() * 1000.0 / f64::from(STEPS));
    }
}
