//! Glass renders the same however a render is cut: a band or a canvas patch whose edge runs
//! through a pane shows exactly the pixels of the whole render.

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Effect, EffectKind, Glass, NodeId, NodeKind};
use figcraft_geom::Affine;
use tiny_skia::Pixmap;

use crate::{Options, View, render_view_single};

/// Stripes under a 600 × 360 pane at (300, 220) with corner `radius` and `g`: (document, page, pane).
fn pane_over_stripes(g: Glass, radius: f64) -> (Document, NodeId, NodeId) {
    let mut d = Document::new("glass");
    let p = d.first_page().unwrap();
    let colours = ["E03030", "2040E0", "20C060", "F0C830"].map(|h| Color::from_hex(h).unwrap());
    for i in 0..24u32 {
        let s = d.create(p, NodeKind::Rectangle, "Stripe", f64::from(i) * 50.0, 0.0, 50.0, 800.0).unwrap();
        d.get_mut(s).unwrap().fills = vec![Paint::solid(colours[i as usize % 4])];
    }
    let pane = d.create(p, NodeKind::Rectangle, "Glass", 0.0, 0.0, 600.0, 360.0).unwrap();
    let n = d.get_mut(pane).unwrap();
    n.transform = Affine::translate((300.0, 220.0));
    n.corners.radii = [radius; 4];
    n.fills = vec![Paint::solid(Color { a: 0.1, ..Color::WHITE })];
    n.effects = vec![Effect { kind: EffectKind::Glass(g), visible: true }];
    (d, p, pane)
}

/// Rows `y0..` of `full` (as wide as `part`) equal `part`, byte for byte: the largest channel
/// difference and how many bytes differ.
fn diff_rows(full: &Pixmap, y0: u32, part: &Pixmap) -> (u8, usize) {
    let row = full.width() as usize * 4;
    let a = &full.data()[y0 as usize * row..][..part.data().len()];
    a.iter().zip(part.data()).fold((0, 0), |(m, n), (x, y)| (m.max(x.abs_diff(*y)), n + usize::from(x != y)))
}

/// A strong, wide light and nothing that blurs or bends: the light reaches furthest in, and the
/// backdrop effects read nothing past the layer, so a cut through the pane gets no margin.
fn wide_light() -> Glass {
    Glass { splay: 1.0, frost: 0.0, refraction: 0.0, light_intensity: 1.0, ..Glass::default() }
}

/// The largest channel difference and bytes differing between the whole render of `d` and renders
/// cut 50 to 80 px inside the pane's top edge (y = 440 px), as bands and canvas patches cut it:
/// rendered with the backdrop reach as a margin above and below, keeping 400 rows from the cut.
fn cut_diffs(d: &Document, p: NodeId) -> Vec<(u32, (u8, usize))> {
    let opts = Options::default();
    let full = render_view_single(d, p, &View { origin: (0.0, 0.0), scale: 2.0, width: 2400, height: 1600 }, &opts).unwrap();
    let margin = crate::backdrop_reach(crate::max_backdrop_blur(d), 2.0).ceil() as u32 + 1;
    [490u32, 505, 520]
        .into_iter()
        .map(|cut| {
            let top = cut - margin;
            let v = View { origin: (0.0, f64::from(top) / 2.0), scale: 2.0, width: 2400, height: 400 + 2 * margin };
            let part = render_view_single(d, p, &v, &opts).unwrap();
            let kept = part.clone_rect(tiny_skia::IntRect::from_xywh(0, margin as i32, 2400, 400).unwrap()).unwrap();
            (cut, diff_rows(&full, cut, &kept))
        })
        .collect()
}

/// A deep pane over black, lit from the left, at 2× (its 1-unit bevel 2 px wide): across its middle
/// row the left rim catches the light, the right rim 0.6 as much, and in between only the faint
/// gradient across the curve shows (black can't darken, so the lit half stays black), changing
/// smoothly, with no ring.
#[test]
fn the_light_is_a_line_on_the_bevel() {
    let mut d = Document::new("glass");
    let p = d.first_page().unwrap();
    let back = d.create(p, NodeKind::Rectangle, "Back", 0.0, 0.0, 700.0, 700.0).unwrap();
    d.get_mut(back).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    let pane = d.create(p, NodeKind::Rectangle, "Glass", 50.0, 50.0, 600.0, 600.0).unwrap();
    let g = Glass { depth: 300.0, light_angle: -90.0, light_intensity: 1.0, frost: 0.0, refraction: 0.0, ..Glass::default() };
    let n = d.get_mut(pane).unwrap();
    n.fills.clear();
    n.effects = vec![Effect { kind: EffectKind::Glass(g), visible: true }];
    let full = render_view_single(&d, p, &View { origin: (0.0, 0.0), scale: 2.0, width: 1400, height: 1400 }, &Options::default()).unwrap();
    let row = &full.data()[700 * 1400 * 4..][..1400 * 4];
    let red: Vec<u8> = row.chunks(4).map(|px| px[0]).collect();
    // The pane covers pixels 100..1300 of the row.
    let (left, right) = (red[100], red[1299]);
    assert!(left > 230, "the left rim catches the light, less the gradient: {left}");
    assert!((140..=175).contains(&right), "the right rim 0.6 as much, plus the gradient: {right}");
    assert!(red[106] <= 14 && red[1293] <= 14, "3 units in, the gradient only: {} {}", red[106], red[1293]);
    let step = red[104..1296].windows(2).map(|w| w[0].abs_diff(w[1])).max().unwrap();
    assert!(step <= 2, "the gradient jumps {step} levels between neighbouring pixels");
}

/// Digests of glass renders over a spread of settings, scales and corners: compare two builds'
/// output with `cargo test --release -p figcraft-render --lib glass_digests -- --ignored --nocapture`.
#[test]
#[ignore = "prints digests to compare builds"]
fn glass_digests() {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let looks = [
        Glass::default(),
        wide_light(),
        Glass { splay: 0.5, frost: 0.0, refraction: 0.0, ..Glass::default() },
        Glass { refraction: 1.0, dispersion: 1.0, frost: 10.0, depth: 60.0, light_intensity: 0.0, ..Glass::default() },
        Glass { depth: 1.0, splay: 0.3, light_angle: 120.0, ..Glass::default() },
    ];
    let mut all = DefaultHasher::new();
    for (i, g) in looks.into_iter().enumerate() {
        for radius in [0.0, 40.0] {
            let (d, p, _) = pane_over_stripes(g.clone(), radius);
            for scale in [1.0, 1.37, 2.0] {
                let v = View { origin: (100.0, 100.0), scale, width: (1000.0 * scale) as u32, height: (700.0 * scale) as u32 };
                // Rendered twice: the second reads the whole shape the first asked for.
                let first = render_view_single(&d, p, &v, &Options::default()).unwrap();
                let pix = render_view_single(&d, p, &v, &Options::default()).unwrap();
                assert!(first.data() == pix.data(), "glass {i} radius {radius} scale {scale}: the kept shape renders differently");
                let mut h = DefaultHasher::new();
                pix.data().hash(&mut h);
                println!("glass {i} radius {radius} scale {scale}: {:016x}", h.finish());
                pix.data().hash(&mut all);
            }
        }
    }
    println!("all: {:016x}", all.finish());
}

#[test]
fn a_cut_through_the_pane_matches_the_whole_render() {
    // The last two looks are deep, so the curve reaches far in from every cut: 60 units (at 2× a
    // core 122 px in, which takes the widest margin a core gets) and 300 (too big for a core).
    let looks = [
        wide_light(),
        Glass::default(),
        Glass { splay: 0.5, frost: 0.0, refraction: 0.0, ..Glass::default() },
        // Splay with the bend and dispersion on: the dispersed samples follow the turned bend.
        Glass { splay: 1.0, frost: 0.0, ..Glass::default() },
        Glass { depth: 60.0, ..Glass::default() },
        Glass { depth: 300.0, ..wide_light() },
    ];
    // Rounded corners without the pane's fill: a fill's curved edge rasters 1–2 levels apart in a
    // cut, and that is the fill, not the glass.
    // Plain corners work out in closed form; smoothed ones (sharp ones too: any smoothing counts)
    // from the coverage, the deepest look too big for a core and bending along its normals.
    for (radius, smoothing) in [(0.0, 0.0), (40.0, 0.0), (0.0, 0.6), (40.0, 0.6)] {
        for g in &looks {
            let (mut d, p, pane) = pane_over_stripes(g.clone(), radius);
            d.get_mut(pane).unwrap().corners.smoothing = smoothing;
            if radius > 0.0 {
                d.get_mut(pane).unwrap().fills.clear();
            }
            for (cut, diff) in cut_diffs(&d, p) {
                assert_eq!(diff, (0, 0), "cut at {cut} px, radius {radius}, smoothing {smoothing}, with {g:?}");
            }
        }
    }
}

/// A `w` × `h` pane of glass `g` at (`x`, `y`) over stripes, alone on its page: (document, page,
/// pane). Each test gives its own size, so tests running at once don't share cached shapes. Its
/// corners are smoothed, so its fields come from its coverage and are kept (a plain rounded
/// rectangle's are worked out in closed form, with nothing to keep).
fn pane(g: Glass, x: f64, y: f64, w: f64, h: f64) -> (Document, NodeId, NodeId) {
    let mut d = Document::new("glass");
    let p = d.first_page().unwrap();
    for i in 0..8u32 {
        let s = d.create(p, NodeKind::Rectangle, "Stripe", f64::from(i) * 40.0, 0.0, 20.0, 400.0).unwrap();
        d.get_mut(s).unwrap().fills = vec![Paint::solid(Color::from_hex("2040E0").unwrap())];
    }
    let pane = d.create(p, NodeKind::Rectangle, "Glass", 0.0, 0.0, w, h).unwrap();
    let n = d.get_mut(pane).unwrap();
    n.transform = Affine::translate((x, y));
    n.corners.radii = [12.0; 4];
    n.corners.smoothing = 0.6;
    n.effects = vec![Effect { kind: EffectKind::Glass(g), visible: true }];
    (d, p, pane)
}

fn render(d: &Document, p: NodeId, origin: (f64, f64), scale: f64) -> Pixmap {
    render_view_single(d, p, &View { origin, scale, width: 400, height: 300 }, &Options::default()).unwrap()
}

/// How many glass shapes `f` works out on this thread.
fn computed(f: impl FnOnce()) -> u64 {
    let before = crate::glass::glass_fields_computed();
    f();
    crate::glass::glass_fields_computed() - before
}

/// How many glass shapes, and how many shapes' bend directions towards their core, `f` works out
/// on this thread.
fn computed_and_bent(f: impl FnOnce()) -> (u64, u64) {
    let before = (crate::glass::glass_fields_computed(), crate::glass::glass_bends_computed());
    f();
    (crate::glass::glass_fields_computed() - before.0, crate::glass::glass_bends_computed() - before.1)
}

#[test]
fn kept_fields_render_the_same_pixels_as_fresh_ones() {
    let (d, p, _) = pane(Glass::default(), 40.0, 30.0, 211.0, 137.0);
    let mut shots = Vec::new();
    // The first render works out the part it shows, the next the whole shape (and keeps it), the
    // ones after reuse it.
    for want in [1, 1, 0] {
        assert_eq!(computed(|| shots.push(render(&d, p, (0.0, 0.0), 1.5))), want, "render {}", shots.len() + 1);
    }
    assert!(shots.windows(2).all(|w| w[0].data() == w[1].data()));
}

#[test]
fn what_the_shape_depends_on_works_it_out_again_and_nothing_else_does() {
    let (mut d, p, pane) = pane(Glass::default(), 40.0, 30.0, 223.0, 149.0);
    let set = |d: &mut Document, g: Glass| d.get_mut(pane).unwrap().effects = vec![Effect { kind: EffectKind::Glass(g), visible: true }];
    // Asked for twice: kept.
    let _ = render(&d, p, (0.0, 0.0), 1.0);
    let _ = render(&d, p, (0.0, 0.0), 1.0);
    // Reused: the look apart from depth, what lies under the glass, panning by whole pixels and
    // moving the pane by whole pixels.
    let reused = computed(|| {
        set(&mut d, Glass { light_angle: 30.0, refraction: 0.2, dispersion: 1.0, frost: 9.0, ..Glass::default() });
        let _ = render(&d, p, (0.0, 0.0), 1.0);
        let stripe = d.node(p).unwrap().children[0];
        d.get_mut(stripe).unwrap().transform = Affine::translate((5.0, 0.0));
        let _ = render(&d, p, (0.0, 0.0), 1.0);
        let _ = render(&d, p, (7.0, -3.0), 1.0);
        d.get_mut(pane).unwrap().transform = Affine::translate((52.0, 31.0));
        let _ = render(&d, p, (0.0, 0.0), 1.0);
    });
    assert_eq!(reused, 0);
    // Worked out again: another zoom, a move by part of a pixel, another shape, a much deeper edge.
    assert_eq!(computed(|| drop(render(&d, p, (0.0, 0.0), 1.25))), 1, "zoom");
    d.get_mut(pane).unwrap().transform = Affine::translate((52.5, 31.0));
    assert_eq!(computed(|| drop(render(&d, p, (0.0, 0.0), 1.0))), 1, "part of a pixel");
    d.get_mut(pane).unwrap().width = 230.0;
    assert_eq!(computed(|| drop(render(&d, p, (0.0, 0.0), 1.0))), 1, "shape");
    let _ = render(&d, p, (0.0, 0.0), 1.0);
    // A depth a unit more keeps the same margin: the kept shape is reused, and only the bend
    // towards the core, which moves with the depth, is worked out again, once (a Depth slider
    // dragged works out nothing else).
    set(&mut d, Glass { depth: 21.0, ..Glass::default() });
    assert_eq!(computed_and_bent(|| drop(render(&d, p, (0.0, 0.0), 1.0))), (0, 1), "depth, within the same margin");
    assert_eq!(computed_and_bent(|| drop(render(&d, p, (0.0, 0.0), 1.0))), (0, 0), "the same depth again");
    set(&mut d, Glass { depth: 70.0, ..Glass::default() });
    assert_eq!(computed(|| drop(render(&d, p, (0.0, 0.0), 1.0))), 1, "depth");
}

#[test]
fn a_pane_too_big_to_keep_still_renders() {
    // 400 × 300 units at 8x: far past what is kept whole, so each render works out its part.
    let (d, p, _) = pane(Glass { frost: 0.0, ..Glass::default() }, 0.0, 0.0, 400.0, 300.0);
    let shown = render(&d, p, (10.0, 10.0), 8.0);
    assert_eq!(computed(|| drop(render(&d, p, (10.0, 10.0), 8.0))), 1);
    assert!(shown.data().chunks_exact(4).any(|px| px[2] > 200), "the stripes show through");
}
