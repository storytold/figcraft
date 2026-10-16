//! A drop shadow under a layer with a background blur (#255): the shadow is cast by what the layer
//! paints (a translucent fill casts a faint one), not by the blurred backdrop drawn through its
//! shape, and the blur reads what lies under the layer with its own shadow there, so a shadow
//! that shows behind transparent areas is seen, blurred, through the layer.

use figcraft_color::{Color, Paint};
use figcraft_doc::{Blur, Document, Effect, EffectKind, NodeId, NodeKind, Shadow};

use super::*;

/// A white 240 × 200 frame with blue and green stripes and, on top, a pane at (60, 40) 120 × 80
/// filled 20% black, with a hard black drop shadow 40 down (below it and under its lower half)
/// and, when `blur`, a background blur.
fn scene(show_behind: bool, blur: bool) -> (Document, NodeId) {
    let mut d = Document::new("shadow behind blur");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "Frame", 0.0, 0.0, 240.0, 200.0).unwrap();
    d.get_mut(f).unwrap().fills = vec![Paint::solid(Color::from_hex("FFFFFF").unwrap())];
    for (i, x) in [20.0, 80.0, 140.0, 200.0].into_iter().enumerate() {
        let s = d.create(f, NodeKind::Rectangle, "Stripe", x, 0.0, 20.0, 200.0).unwrap();
        d.get_mut(s).unwrap().fills = vec![Paint::solid(Color::from_hex(if i % 2 == 0 { "2040E0" } else { "20C060" }).unwrap())];
    }
    let pane = d.create(f, NodeKind::Rectangle, "Pane", 60.0, 40.0, 120.0, 80.0).unwrap();
    let n = d.get_mut(pane).unwrap();
    n.fills = vec![Paint::solid(Color::new(0.0, 0.0, 0.0, 0.2))];
    let shadow = Shadow { color: Color::new(0.0, 0.0, 0.0, 1.0), offset: (0.0, 40.0), radius: 0.0, show_behind, ..Shadow::default() };
    n.effects = vec![Effect { kind: EffectKind::DropShadow(shadow), visible: true }];
    if blur {
        n.effects.push(Effect { kind: EffectKind::BackgroundBlur(Blur::uniform(8.0)), visible: true });
    }
    (d, f)
}

fn px(p: &Pixmap, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width() + x) * 4) as usize;
    [p.data()[i], p.data()[i + 1], p.data()[i + 2], p.data()[i + 3]]
}

/// The largest channel difference between `a` and `b` over the shadow below the pane
/// (60..180 × 122..160).
fn max_diff_below_pane(a: &Pixmap, b: &Pixmap) -> u8 {
    let mut worst = 0;
    for y in 122..160 {
        for x in 60..180 {
            worst = worst.max(px(a, x, y).iter().zip(px(b, x, y)).map(|(c, d)| c.abs_diff(d)).max().unwrap());
        }
    }
    worst
}

#[test]
fn a_background_blur_doesnt_change_the_shadow_its_layer_casts() {
    for show_behind in [false, true] {
        let (with_blur, f) = scene(show_behind, true);
        let (without, _) = scene(show_behind, false);
        let got = render_node(&with_blur, f, 1.0).unwrap();
        let want = render_node(&without, f, 1.0).unwrap();
        let diff = max_diff_below_pane(&got, &want);
        assert!(diff <= 2, "show behind {show_behind}: the shadow below the pane differs by {diff}");
        // A 20% fill casts a faint shadow: the white between the stripes stays light.
        let light = px(&got, 120, 140);
        assert!(light[0] > 180, "show behind {show_behind}: {light:?}");
    }
}

#[test]
fn a_shadow_showing_behind_is_seen_through_the_background_blur() {
    let (d, f) = scene(true, true);
    let got = render_node(&d, f, 1.0).unwrap();
    // Between the stripes, the pane's top (no shadow under it) and its lower half (its own shadow
    // under it, from y = 80).
    let (top, low) = (px(&got, 120, 50), px(&got, 120, 110));
    assert!(top[0] > low[0] + 20, "the shadow under the pane doesn't show through: top {top:?}, lower half {low:?}");
    // Hidden behind the layer, it doesn't.
    let (d, f) = scene(false, true);
    let got = render_node(&d, f, 1.0).unwrap();
    let (top, low) = (px(&got, 120, 50), px(&got, 120, 110));
    assert!(top[0].abs_diff(low[0]) <= 2, "a hidden shadow shows: top {top:?}, lower half {low:?}");
}
