//! Drop shadows cast from a layer's outline ([`crate::shadow::outline_caster`]) look as the ones
//! cast from its pixels do, in every piece a view is rendered in, and cost what shows of them.

use figcraft_color::{Color, Paint};
use figcraft_doc::{EffectKind, NodeId, Shadow};

use crate::shadow::FROM_PIXELS;
use crate::zoom_bench_tests::{Card, SCALE, card, close_up};
use crate::{Options, Pixmap, View, render_band, render_view_single};

fn white() -> Options {
    Options { background: Some(Color::WHITE), ..Default::default() }
}

/// The view rendered with shadows cast from outlines, and from pixels (as they all were).
fn both(c: &Card, v: &View) -> (Pixmap, Pixmap) {
    let new = render_view_single(&c.doc, c.page, v, &white()).unwrap();
    FROM_PIXELS.with(|f| f.set(true));
    let old = render_view_single(&c.doc, c.page, v, &white());
    FROM_PIXELS.with(|f| f.set(false));
    (new, old.unwrap())
}

/// The largest difference of a channel between two renders of view `v`, under the card: where
/// only its shadow paints. (The card's own edges differ a little with where a layer's pixmap
/// ends, whatever casts the shadow: curves are flattened from where the pixmap cuts them.)
fn furthest(v: &View, a: &Pixmap, b: &Pixmap) -> u8 {
    let from = (((CARD_BOTTOM - v.origin.1) * v.scale).ceil() as usize + 2) * v.width as usize * 4;
    assert!(from < a.data().len(), "the view shows what is under the card");
    a.data().iter().zip(b.data()).skip(from).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
}

/// The card's bottom edge on the page.
const CARD_BOTTOM: f64 = 465.0;

fn shadow_of(c: &mut Card, f: impl FnOnce(&mut Shadow)) {
    for e in &mut c.doc.get_mut(c.card).unwrap().effects {
        if let EffectKind::DropShadow(s) = &mut e.kind {
            f(s);
            return;
        }
    }
}

/// The card's bottom-left corner and the shadow under it, `scale` device pixels a unit.
fn corner(scale: f64) -> View {
    View { origin: (480.0, 440.0), scale, width: (50.0 * scale) as u32, height: (40.0 * scale) as u32 }
}

fn darkest(p: &Pixmap) -> u8 {
    p.data().as_chunks::<4>().0.iter().map(|c| c[0]).min().unwrap_or(255)
}

#[test]
fn a_shadow_blurred_little_is_the_one_cast_from_the_layers_pixels() {
    let mut c = card(true);
    // A blur of 3 device pixels: drawn at full resolution, a dark shadow so it shows.
    shadow_of(&mut c, |s| (s.radius, s.color) = (6.0, Color { a: 0.8, ..Color::BLACK }));
    for scale in [1.0, 1.37] {
        let (new, old) = both(&c, &corner(scale));
        assert!(darkest(&new) < 120, "a shadow shows at {scale}");
        assert_eq!(furthest(&corner(scale), &new, &old), 0, "at {scale}");
    }
}

#[test]
fn a_shadow_blurred_widely_looks_as_the_one_cast_from_the_layers_pixels() {
    for (spread, offset, behind) in [(0.0, (0.0, 4.0), false), (3.0, (2.5, 4.0), false), (-2.0, (0.0, 6.0), false), (0.0, (0.0, 4.0), true)] {
        let mut c = card(true);
        shadow_of(&mut c, |s| {
            (s.spread, s.offset, s.show_behind, s.color) = (spread, offset, behind, Color { a: 0.8, ..Color::BLACK });
        });
        // A frame's shadow spreads from its outline when it clips (from its pixels otherwise).
        c.doc.get_mut(c.card).unwrap().clip = spread != 0.0;
        if behind {
            // What shows of a shadow behind its layer: through a see-through picture, not the card.
            let n = c.doc.get_mut(c.card).unwrap();
            n.fills = vec![Paint::solid(Color::WHITE)];
        }
        // Blurs of 16 to 309 device pixels: drawn 4 to 64 times smaller.
        for scale in [4.0, 9.3, SCALE] {
            let v = if scale == SCALE { View { origin: (488.0, 452.0), scale, width: 700, height: 900 } } else { corner(scale) };
            let (new, old) = both(&c, &v);
            assert!(darkest(&new) < 200, "a shadow shows at {scale}");
            let d = furthest(&v, &new, &old);
            assert!(d <= 4, "spread {spread}, offset {offset:?}, behind {behind}, at {scale}: {d} apart");
        }
    }
}

#[test]
fn bands_and_one_piece_draw_the_same_wide_shadow() {
    let mut c = card(true);
    shadow_of(&mut c, |s| s.color = Color { a: 0.8, ..Color::BLACK });
    for scale in [9.3, SCALE] {
        let v = View { origin: (488.3, 452.7), scale, width: 640, height: 600 };
        let whole = render_view_single(&c.doc, c.page, &v, &white()).unwrap();
        let mut bands = Pixmap::new(v.width, v.height).unwrap();
        for y0 in (0..v.height).step_by(75) {
            let part = render_band(&c.doc, c.page, &v, &white(), y0, 75, 0).unwrap();
            crate::put_band(&mut bands, y0, &part);
        }
        assert_eq!(furthest(&v, &bands, &whole), 0, "at {scale}");
    }
}

#[test]
fn a_layer_with_a_shadow_holds_no_more_than_the_view() {
    let c = card(true);
    let v = close_up(&c, 1200, 800);
    let view_bytes = u64::from(v.width) * u64::from(v.height) * 4;
    let (pix, peak) = crate::layer_budget::within(crate::layer_budget::BUDGET, || render_view_single(&c.doc, c.page, &v, &Options::default()));
    assert!(pix.is_some());
    // The card's layer, and the clips inside and around it: each the view at most. Cast from the
    // layer's pixels, the layer alone was the view and 1,392 pixels more each way.
    assert!(peak <= view_bytes * 6, "{} MB held for a view of {} MB", peak >> 20, view_bytes >> 20);
}

#[test]
fn layers_that_are_not_their_outline_cast_from_their_pixels() {
    let caster = |f: fn(&mut Card) -> NodeId| {
        let mut c = card(true);
        let id = f(&mut c);
        crate::shadow::outline_caster(&c.doc, c.doc.node(id).unwrap()).is_some()
    };
    assert!(caster(|c| c.card), "an opaque frame whose children stay inside it");
    assert!(caster(|c| {
        c.doc.get_mut(c.card).unwrap().clip = true;
        c.card
    }));
    assert!(!caster(|c| {
        c.doc.get_mut(c.card).unwrap().fills = vec![Paint::solid(Color { a: 0.5, ..Color::WHITE })];
        c.card
    }));
    // A child past the card's edge, or in the corner its rounding cuts off, casts a shadow too.
    assert!(!caster(|c| {
        let picture = c.doc.node(c.shape).unwrap().parent.unwrap();
        c.doc.get_mut(picture).unwrap().transform = figcraft_geom::Affine::translate((300.0, 12.0));
        c.card
    }));
    assert!(!caster(|c| {
        shadow_of(c, |s| s.spread = 2.0);
        c.card
    }));
    assert!(!caster(|c| {
        let picture = c.doc.node(c.shape).unwrap().parent.unwrap();
        c.doc.get_mut(picture).unwrap().transform = figcraft_geom::Affine::translate((1.0, 1.0));
        c.card
    }));
    assert!(caster(|c| {
        let picture = c.doc.node(c.shape).unwrap().parent.unwrap();
        c.doc.get_mut(picture).unwrap().transform = figcraft_geom::Affine::translate((1.0, 1.0));
        c.doc.get_mut(c.card).unwrap().clip = true;
        c.card
    }));
    assert!(!caster(|c| {
        c.doc
            .get_mut(c.card)
            .unwrap()
            .effects
            .push(figcraft_doc::Effect { kind: EffectKind::LayerBlur(figcraft_doc::Blur::uniform(2.0)), visible: true });
        c.card
    }));
}

/// A rectangle with a dark shadow of `radius` and `spread`, drawn through `transform`.
fn slab(transform: figcraft_geom::Affine, size: (f64, f64), radius: f64, spread: f64) -> (figcraft_doc::Document, NodeId) {
    let mut d = figcraft_doc::Document::new("slab");
    let page = d.first_page().unwrap();
    let r = d.create(page, figcraft_doc::NodeKind::Rectangle, "Slab", 0.0, 0.0, size.0, size.1).unwrap();
    let n = d.get_mut(r).unwrap();
    n.transform = transform;
    n.fills = vec![Paint::solid(Color::WHITE)];
    n.effects.push(figcraft_doc::Effect {
        kind: EffectKind::DropShadow(Shadow { radius, spread, offset: (0.0, 0.0), color: Color::BLACK, ..Shadow::default() }),
        visible: true,
    });
    (d, page)
}

/// A spread grows the outline in the layer's own units: stretched six times one way and squeezed
/// as much the other, it reaches six times as far that way, not as far as the scale of an area
/// (here 1) says.
#[test]
fn a_spread_reaches_as_far_as_the_layer_is_stretched() {
    let stretched = figcraft_geom::Affine::translate((120.0, 60.0)) * figcraft_geom::Affine::scale_non_uniform(6.0, 1.0 / 6.0);
    let (d, page) = slab(stretched, (20.0, 240.0), 0.0, 10.0);
    let v = View { origin: (0.0, 0.0), scale: 1.0, width: 360, height: 160 };
    let new = render_view_single(&d, page, &v, &white()).unwrap();
    FROM_PIXELS.with(|f| f.set(true));
    let old = render_view_single(&d, page, &v, &white()).unwrap();
    FROM_PIXELS.with(|f| f.set(false));
    // Black 50 pixels left of the slab (its spread is 60 there), in both.
    let at = |p: &Pixmap, x: usize, y: usize| p.data()[(y * 360 + x) * 4];
    assert_eq!((at(&new, 70, 80), at(&old, 70, 80)), (0, 0), "the shadow 50 pixels out");
    let apart = new.data().iter().zip(old.data()).filter(|(a, b)| a.abs_diff(**b) > 8).count();
    assert!(apart < 400, "{apart} channels apart");
}

/// A shadow across a view as wide as a pixmap gets is drawn: the pixmap it is blurred in is a
/// blur's reach wider still.
#[test]
fn a_shadow_shows_across_the_widest_view() {
    let (d, page) = slab(figcraft_geom::Affine::translate((-100.0, 4.0)), (17_000.0, 10.0), 6.0, 0.0);
    let v = View { origin: (0.0, 0.0), scale: 1.0, width: 16_384, height: 30 };
    let pix = render_view_single(&d, page, &v, &white()).unwrap();
    // Two rows under the slab (white without a shadow), at both ends and in the middle.
    for x in [10usize, 8_000, 16_370] {
        let red = pix.data()[(16 * 16_384 + x) * 4];
        assert!(red < 245, "a shadow under the slab at {x}: {red}");
    }
}
