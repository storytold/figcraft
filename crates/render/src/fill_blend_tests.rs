//! Blend modes of a layer's paints and shadows, against renders of the same synthetic scenes by
//! the reference tool (the expected values are its pixels, within a few levels): a layer in a
//! blend mode of its own (Normal included) is a group, so its fills blend with each other only and
//! then lie over the canvas as one, with or without effects; a pass-through frame's fills blend
//! with what lies under it, shadows or not. Drop shadows blend among themselves, inner shadows
//! with the fills under them. Plus lighter adds the premultiplied colours and combines the alphas
//! as source-over does.

use figcraft_color::{BlendMode, Color, Paint};
use figcraft_doc::{Document, Effect, EffectKind, NodeId, NodeKind, Shadow};

use super::paint_behaviour_tests::{hex, near, page, px, rect, render, solid_at};
use super::*;

const BACK: &str = "F7F8FC";

fn blended(c: &str, opacity: f32, blend: BlendMode) -> Paint {
    Paint { blend, ..solid_at(c, opacity) }
}

/// The fill stack of the reference tool's glass buttons: black and white at 25%, `#444` at 60%
/// plus lighter, `#F8F8F8` at 20% luminosity.
fn stack() -> Vec<Paint> {
    vec![
        solid_at("000000", 0.25),
        solid_at("FFFFFF", 0.25),
        blended("444444", 0.6, BlendMode::PlusLighter),
        blended("F8F8F8", 0.2, BlendMode::Luminosity),
    ]
}

fn shadow(kind: fn(Shadow) -> EffectKind, c: &str, a: f32, offset: (f64, f64), radius: f64, spread: f64, blend: BlendMode) -> Effect {
    let color = Color { a, ..hex(c) };
    Effect { kind: kind(Shadow { color, offset, radius, spread, blend, ..Shadow::default() }), visible: true }
}

/// A faint drop shadow that only makes the layer carry an effect.
fn faint() -> Effect {
    shadow(EffectKind::DropShadow, "000000", 0.02, (0.0, 8.0), 15.0, 0.0, BlendMode::Normal)
}

/// The backdrop: the page filled `BACK` with a blue stripe on the left.
fn backdrop() -> (Document, NodeId) {
    let (mut d, p) = page();
    rect(&mut d, p, (0.0, 0.0, 200.0, 200.0), vec![Paint::solid(hex(BACK))]);
    rect(&mut d, p, (0.0, 0.0, 40.0, 200.0), vec![Paint::solid(hex("2A7FFF"))]);
    (d, p)
}

/// A 160 × 120 layer at (20, 40) of `kind` with `fills` and `effects` over [`backdrop`].
fn scene(kind: NodeKind, blend: BlendMode, fills: Vec<Paint>, effects: Vec<Effect>) -> Pixmap {
    let (mut d, p) = backdrop();
    let id = d.create(p, kind, "L", 20.0, 40.0, 160.0, 120.0).unwrap();
    let n = d.get_mut(id).unwrap();
    (n.fills, n.effects, n.blend) = (fills, effects, blend);
    render(&d)
}

fn shape(fills: Vec<Paint>, effects: Vec<Effect>) -> Pixmap {
    scene(NodeKind::Rectangle, BlendMode::Normal, fills, effects)
}

#[test]
fn a_shapes_fills_blend_among_themselves_with_or_without_effects() {
    for effects in [vec![], vec![faint()]] {
        let pix = shape(stack(), effects.clone());
        assert!(near(px(&pix, 100, 100), [179, 179, 179], 3), "{:?} with {} effects", px(&pix, 100, 100), effects.len());
        assert!(near(px(&pix, 30, 100), [142, 157, 180], 3), "{:?} over blue", px(&pix, 30, 100));
    }
}

#[test]
fn one_plus_lighter_fill_on_a_shape_is_a_plain_fill() {
    for effects in [vec![], vec![faint()]] {
        let pix = shape(vec![blended("444444", 0.6, BlendMode::PlusLighter)], effects);
        assert!(near(px(&pix, 100, 100), [140, 140, 142], 3), "{:?}", px(&pix, 100, 100));
    }
}

#[test]
fn a_pass_through_frames_fills_blend_with_the_backdrop_under_its_shadows() {
    for effects in [vec![], vec![faint()]] {
        let pix = scene(NodeKind::Frame, BlendMode::PassThrough, stack(), effects.clone());
        assert!(near(px(&pix, 100, 100), [245, 245, 247], 3), "{:?} with {} effects", px(&pix, 100, 100), effects.len());
        assert!(near(px(&pix, 30, 100), [153, 191, 249], 3), "{:?} over blue", px(&pix, 30, 100));
    }
}

#[test]
fn drop_shadows_blend_among_themselves_not_with_the_backdrop() {
    let burn = |c, x: f64, spread| shadow(EffectKind::DropShadow, c, 1.0, (x, 0.0), 0.0, spread, BlendMode::PlusDarker);
    let pix = shape(vec![Paint::solid(Color::WHITE)], vec![burn("E8E8E8", 0.0, 4.0), burn("D0D0D0", -6.0, -2.0)]);
    // Both shadows over the blue stripe: `D0 + E8 - FF`, grey; the first alone: its own colour.
    assert!(near(px(&pix, 17, 100), [185, 185, 185], 3), "{:?}", px(&pix, 17, 100));
    assert!(near(px(&pix, 100, 37), [232, 232, 232], 3), "{:?}", px(&pix, 100, 37));
}

#[test]
fn inner_shadows_blend_with_the_fills_under_them() {
    let burn = shadow(EffectKind::InnerShadow, "E6E6E6", 1.0, (0.0, 40.0), 30.0, -10.0, BlendMode::PlusDarker);
    let dodge = shadow(EffectKind::InnerShadow, "282828", 1.0, (0.0, -40.0), 10.0, -10.0, BlendMode::PlusLighter);
    let pix = shape(vec![Paint::solid(hex("808080"))], vec![dodge, burn]);
    // Near the top the burn darkens the grey (`80 + E6 - FF`), near the bottom the dodge lightens it.
    assert!(near(px(&pix, 100, 45), [103, 103, 103], 4), "{:?}", px(&pix, 100, 45));
    assert!(near(px(&pix, 100, 155), [168, 168, 168], 4), "{:?}", px(&pix, 100, 155));
}

/// A pass-through frame inside a pass-through frame: its Multiply fill blends with the backdrop
/// whether or not the frame around it casts shadows (yellow over the light backdrop comes out
/// its yellow, not pure yellow).
#[test]
fn a_nested_pass_through_frames_fills_blend_with_the_backdrop_under_shadows() {
    for effects in [vec![], vec![faint()]] {
        let (mut d, p) = backdrop();
        let outer = d.create(p, NodeKind::Frame, "Outer", 20.0, 40.0, 160.0, 120.0).unwrap();
        let n = d.get_mut(outer).unwrap();
        (n.fills, n.effects, n.blend) = (vec![], effects.clone(), BlendMode::PassThrough);
        let inner = d.create(outer, NodeKind::Frame, "Inner", 0.0, 0.0, 160.0, 120.0).unwrap();
        let n = d.get_mut(inner).unwrap();
        (n.fills, n.blend) = (vec![blended("FFFF00", 1.0, BlendMode::Multiply)], BlendMode::PassThrough);
        let pix = render(&d);
        assert!(near(px(&pix, 100, 100), [247, 248, 0], 3), "{:?} with {} effects", px(&pix, 100, 100), effects.len());
    }
}
