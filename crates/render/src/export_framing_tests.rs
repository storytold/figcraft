//! How an export frames a layer, as the reference tool's exports do (measured on its renders of the
//! same scenes): a drop shadow counts as far as its blur radius (its faint tail past that stays
//! out), and a layer inside that draws nothing doesn't count.

use figcraft_color::{BlendMode, Color, Paint};
use figcraft_doc::{Effect, EffectKind, NodeKind, Shadow};

use super::paint_behaviour_tests::page;
use crate::export_bounds;

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
