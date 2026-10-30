//! Backdrop effects (background blur, glass) see the whole backdrop under the layer: everything
//! composited below it, ancestors' own fills included, however the layer's ancestors isolate it
//! (clipping frames, rounded clips, group opacity, masks). The expected composite is built here
//! from first principles: the scene rendered without the layer, blurred, through the shape.

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Effect, EffectKind, Glass, NodeId, NodeKind};
use tiny_skia::{PixmapPaint, Transform};

use super::*;

const BLUR: f64 = 12.0;

fn solid(d: &mut Document, id: NodeId, hex: &str) {
    d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex(hex).unwrap())];
}

/// A 240 × 160 red frame (clipping) with blue and green stripes inside and, on top, a transparent
/// pane with a background blur at (60, 40) 120 × 80 under `holder` (what isolates it). Returns the
/// document, the frame and the pane.
fn scene(holder: Holder) -> (Document, NodeId, NodeId) {
    let mut d = Document::new("backdrop");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "Frame", 0.0, 0.0, 240.0, 160.0).unwrap();
    solid(&mut d, f, "E03030");
    d.get_mut(f).unwrap().clip = true;
    if holder == Holder::RoundedClip {
        d.get_mut(f).unwrap().corners.radii = [24.0; 4];
    }
    let parent = match holder {
        Holder::Group => {
            let g = d.create(f, NodeKind::Group, "Group", 0.0, 0.0, 240.0, 160.0).unwrap();
            d.get_mut(g).unwrap().opacity = 0.5;
            g
        }
        _ => f,
    };
    for (i, x) in [30.0, 90.0, 150.0].into_iter().enumerate() {
        let s = d.create(parent, NodeKind::Rectangle, "Stripe", x, 0.0, 30.0, 160.0).unwrap();
        solid(&mut d, s, if i % 2 == 0 { "2040E0" } else { "20C060" });
    }
    if holder == Holder::Mask {
        // A mask covering the frame: everything above it shows through its alpha.
        let m = d.create(f, NodeKind::Rectangle, "Mask", 0.0, 0.0, 240.0, 160.0).unwrap();
        solid(&mut d, m, "000000");
        d.get_mut(m).unwrap().is_mask = true;
        let c = d.get_mut(f).unwrap();
        let mi = c.children.iter().position(|k| *k == m).unwrap();
        let mid = c.children.remove(mi);
        c.children.insert(0, mid);
    }
    let pane = d.create(parent, NodeKind::Rectangle, "Pane", 60.0, 40.0, 120.0, 80.0).unwrap();
    let n = d.get_mut(pane).unwrap();
    n.fills = vec![];
    n.effects = vec![Effect { kind: EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(BLUR)), visible: true }];
    (d, f, pane)
}

#[derive(Clone, Copy, PartialEq)]
enum Holder {
    ClipFrame,
    RoundedClip,
    Group,
    Mask,
}

fn px(p: &Pixmap, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width() + x) * 4) as usize;
    [p.data()[i], p.data()[i + 1], p.data()[i + 2], p.data()[i + 3]]
}

/// The largest channel difference between `a` and `b` over the pixels well inside the pane
/// (60..180 × 40..120, 2 px in), at `scale`.
fn max_diff_in_pane(a: &Pixmap, b: &Pixmap, scale: f64) -> u8 {
    let r = |v: f64| (v * scale) as u32;
    let mut worst = 0;
    for y in r(42.0)..r(118.0) {
        for x in r(62.0)..r(178.0) {
            let (p, q) = (px(a, x, y), px(b, x, y));
            worst = worst.max(p.iter().zip(q).map(|(c, d)| c.abs_diff(d)).max().unwrap());
        }
    }
    worst
}

/// The largest channel difference between two same-sized pixmaps.
fn max_diff(a: &Pixmap, b: &Pixmap) -> u8 {
    a.data().iter().zip(b.data()).map(|(p, q)| p.abs_diff(*q)).max().unwrap_or(0)
}

/// The scene's frame exported without the pane, blurred (sigma = radius / 2): what the pane
/// must show.
fn expected_blurred(d: &Document, f: NodeId, pane: NodeId, scale: f64) -> (Pixmap, Pixmap) {
    let mut without = d.clone();
    without.get_mut(pane).unwrap().visible = false;
    let back = render_node(&without, f, scale).unwrap();
    let mut blurred = back.clone();
    fx::blur(&mut blurred, BLUR / 2.0 * scale, false);
    (back, blurred)
}

#[test]
fn background_blur_in_a_clipping_frame_sees_the_frames_fill() {
    for holder in [Holder::ClipFrame, Holder::RoundedClip, Holder::Mask] {
        for scale in [1.0, 2.0] {
            let (d, f, pane) = scene(holder);
            let got = render_node(&d, f, scale).unwrap();
            let (_, want) = expected_blurred(&d, f, pane, scale);
            let diff = max_diff_in_pane(&got, &want, scale);
            assert!(
                diff <= 3,
                "{} at {scale}×: differs by {diff}",
                match holder {
                    Holder::ClipFrame => "clip frame",
                    Holder::RoundedClip => "rounded clip",
                    Holder::Mask => "mask",
                    Holder::Group => "group",
                }
            );
            // The frame's red shows through the blur between the stripes (it used to be missing,
            // the sharp stripes below showing instead).
            let mid = px(&got, (75.0 * scale) as u32, (80.0 * scale) as u32);
            assert!(mid[0] > 80 && mid[3] == 255, "{mid:?}");
        }
    }
}

#[test]
fn background_blur_inside_a_faded_group_blends_with_the_group() {
    let (d, f, pane) = scene(Holder::Group);
    let got = render_node(&d, f, 1.0).unwrap();
    let (_, blurred) = expected_blurred(&d, f, pane, 1.0);
    // The pane (the blurred backdrop, opaque) lives in the group, which is half transparent over
    // the frame: half the blurred backdrop over the frame alone.
    let group = d.node(pane).unwrap().parent.unwrap();
    let mut frame_only = d.clone();
    frame_only.get_mut(group).unwrap().visible = false;
    let mut want = render_node(&frame_only, f, 1.0).unwrap();
    want.draw_pixmap(0, 0, blurred.as_ref(), &PixmapPaint { opacity: 0.5, ..Default::default() }, Transform::identity(), None);
    let diff = max_diff_in_pane(&got, &want, 1.0);
    assert!(diff <= 3, "differs by {diff}");
}

#[test]
fn background_blur_in_a_clipping_frame_inside_a_faded_group() {
    // The clipping frame's children are drawn on a copy of the group's pixels: the pane's
    // backdrop is that copy where the group puts it, at the group's 50% (half the frame), not the
    // copy at full strength.
    let (mut d, f, pane) = scene(Holder::ClipFrame);
    let p = d.first_page().unwrap();
    let g = d.create(p, NodeKind::Group, "Faded", 0.0, 0.0, 240.0, 160.0).unwrap();
    d.get_mut(g).unwrap().opacity = 0.5;
    d.reparent(f, g, None).unwrap();
    let got = render_node(&d, g, 1.0).unwrap();
    // The pane draws the blurred half-strength frame over the frame, and the group halves both.
    let (frame, blurred) = expected_blurred(&d, f, pane, 1.0);
    let mut want = Pixmap::new(frame.width(), frame.height()).unwrap();
    want.draw_pixmap(0, 0, frame.as_ref(), &PixmapPaint { opacity: 0.25, ..Default::default() }, Transform::identity(), None);
    let plus = PixmapPaint { opacity: 0.25, blend_mode: tiny_skia::BlendMode::Plus, ..Default::default() };
    want.draw_pixmap(0, 0, blurred.as_ref(), &plus, Transform::identity(), None);
    let diff = max_diff_in_pane(&got, &want, 1.0);
    assert!(diff <= 3, "differs by {diff}");
}

#[test]
fn background_blur_on_the_canvas_matches_the_export() {
    let (d, f, _) = scene(Holder::ClipFrame);
    let p = d.first_page().unwrap();
    let export = render_node(&d, f, 1.0).unwrap();
    let canvas = render_view(&d, p, &View { origin: (0.0, 0.0), scale: 1.0, width: 240, height: 160 }, &Options::default()).unwrap();
    assert!(max_diff_in_pane(&export, &canvas, 1.0) <= 1);
}

#[test]
fn background_blur_reads_past_its_own_box() {
    // The pane's own border pixels blur what lies around them (the backdrop past the pane's box),
    // not transparency: they match the blurred scene like the inside does.
    let (d, f, pane) = scene(Holder::ClipFrame);
    let got = render_node(&d, f, 1.0).unwrap();
    let (_, want) = expected_blurred(&d, f, pane, 1.0);
    for (x, y) in [(60, 80), (179, 80), (120, 40), (120, 119), (60, 40), (179, 119)] {
        let (g, w) = (px(&got, x, y), px(&want, x, y));
        assert!(g.iter().zip(w).all(|(a, b)| a.abs_diff(b) <= 3), "({x}, {y}): {g:?} vs {w:?}");
    }
}

/// The frame with a glass pane at 80..200 × 40..120 instead of the blur: (document, frame, pane).
fn glass_scene(g: Glass) -> (Document, NodeId, NodeId) {
    let (mut d, f, pane) = scene(Holder::ClipFrame);
    let n = d.get_mut(pane).unwrap();
    n.effects = vec![Effect { kind: EffectKind::Glass(g), visible: true }];
    // 80..200 × 40..120: stripe edges (x = 90, 150, 180) run through its rims.
    n.transform = Affine::translate((80.0, 40.0));
    (d, f, pane)
}

#[test]
fn glass_centre_is_the_frosted_backdrop() {
    let g = Glass { frost: BLUR, depth: 10.0, ..Glass::default() };
    let (d, f, pane) = glass_scene(g);
    let got = render_node(&d, f, 1.0).unwrap();
    let (_, want) = expected_blurred(&d, f, pane, 1.0);
    // Further than `depth` from the edge nothing bends and no light falls.
    let mut worst = 0;
    for y in 55..105 {
        for x in 95..185 {
            let (p, q) = (px(&got, x, y), px(&want, x, y));
            worst = worst.max(p.iter().zip(q).map(|(c, d)| c.abs_diff(d)).max().unwrap());
        }
    }
    assert!(worst <= 3, "{worst}");
}

#[test]
fn glass_edges_bend_split_and_catch_the_light() {
    let plain = Glass { frost: 0.0, refraction: 0.0, dispersion: 0.0, light_intensity: 0.0, depth: 16.0, ..Glass::default() };
    let (d, f, _) = glass_scene(plain.clone());
    let flat = render_node(&d, f, 1.0).unwrap();
    // Without bend, frost or light the glass shows the backdrop as is.
    let (d0, f0, p0) = glass_scene(plain.clone());
    let mut none = d0.clone();
    none.get_mut(p0).unwrap().visible = false;
    let backdrop = render_node(&none, f0, 1.0).unwrap();
    assert!(max_diff_in_pane(&flat, &backdrop, 1.0) <= 2);

    // Refraction moves what shows near the edge (stripe edges within `depth` of the pane's
    // border get pulled inwards), not in the middle.
    let (d, f, _) = glass_scene(Glass { refraction: 1.0, ..plain.clone() });
    let bent = render_node(&d, f, 1.0).unwrap();
    // Pixels of the pane that differ between two renders, near the border (within 15 px) and in
    // the middle (more than 17 px in).
    let changed = |a: &Pixmap, b: &Pixmap| {
        let (mut rim, mut middle) = (0, 0);
        for y in 40..120u32 {
            for x in 80..200u32 {
                let edge = (x - 80).min(199 - x).min(y - 40).min(119 - y);
                if px(a, x, y) != px(b, x, y) {
                    if edge < 15 {
                        rim += 1;
                    } else if edge > 17 {
                        middle += 1;
                    }
                }
            }
        }
        (rim, middle)
    };
    let (rim, middle) = changed(&bent, &flat);
    assert!(rim > 10 && middle == 0, "rim {rim}, middle {middle}");

    // Dispersion splits the channels along the rim (a fringe on the stripe edges).
    let (d, f, _) = glass_scene(Glass { refraction: 1.0, dispersion: 1.0, ..plain.clone() });
    let split = render_node(&d, f, 1.0).unwrap();
    let (rim, middle) = changed(&split, &bent);
    assert!(rim > 0 && middle == 0, "dispersion: rim {rim}, middle {middle}");

    // The light (from the top left by default) brightens the top-left rim more than the
    // bottom-right one, and only near the edge.
    let (d, f, _) = glass_scene(Glass { light_intensity: 1.0, ..plain });
    let lit = render_node(&d, f, 1.0).unwrap();
    let gain = |x: u32, y: u32| i32::from(px(&lit, x, y)[1]) - i32::from(px(&flat, x, y)[1]);
    assert!(gain(82, 42) > gain(197, 117), "{} vs {}", gain(82, 42), gain(197, 117));
    assert!(gain(82, 42) > 40, "{}", gain(82, 42));
    assert_eq!(gain(140, 80), 0);
}

#[test]
fn glass_and_backdrop_blur_survive_bands_and_partial_renders() {
    let (d, _, _) = glass_scene(Glass::default());
    let p = d.first_page().unwrap();
    let v = View { origin: (-20.0, -20.0), scale: 1.0, width: 280, height: 400 };
    let one = render_view_single(&d, p, &v, &Options::default()).unwrap();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let banded = render_bands(&d, p, &v, &Options::default(), 4).unwrap();
        assert!(max_diff(&one, &banded) <= 2);
    }
    // A pan: the retained render patches the uncovered strip and still matches a full render.
    let doc = std::sync::Arc::new(d);
    let mut r = Retained::default();
    r.render(&doc, p, &v, &Options::default()).unwrap();
    let moved = View { origin: (-12.0, -20.0), ..v.clone() };
    let (snapped, partial, _) = r.render(&doc, p, &moved, &Options::default()).unwrap();
    let partial = partial.clone();
    let full = render_view_single(&doc, p, &snapped, &Options::default()).unwrap();
    assert!(max_diff(&partial, &full) <= 2);
}

#[test]
fn hostile_backdrop_values_do_not_crash() {
    for g in [
        Glass { depth: f64::NAN, frost: f64::INFINITY, refraction: -3.0, dispersion: 9.0, light_angle: f64::NAN, ..Glass::default() },
        Glass { depth: 1e12, frost: 1e9, ..Glass::default() },
        Glass { depth: 0.0, frost: 0.0, refraction: 0.0, light_intensity: 0.0, ..Glass::default() },
    ] {
        let (d, f, _) = glass_scene(g);
        assert!(render_node(&d, f, 1.0).is_some());
    }
    let (mut d, f, pane) = scene(Holder::ClipFrame);
    d.get_mut(pane).unwrap().effects = vec![Effect { kind: EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(f64::NAN)), visible: true }];
    assert!(render_node(&d, f, 1.0).is_some());
    d.get_mut(pane).unwrap().effects = vec![Effect { kind: EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(1e12)), visible: true }];
    assert!(render_node(&d, f, 0.5).is_some());
}
