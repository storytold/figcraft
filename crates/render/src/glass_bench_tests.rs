//! Glass render timings: a 600 × 360 rounded glass pane over coloured stripes, 2400 × 1600 device
//! pixels. Run in release:
//! `cargo test --release -p figcraft-render --lib glass_render_timings -- --ignored --nocapture`.
//!
//! "export" rows render the whole view with `render_view` (exports, thumbnails, the first canvas
//! frame); "canvas" rows go through `Retained`, which re-renders only what changed, as the canvas
//! does; "single" renders without parallel bands (as on the web).

use std::sync::Arc;
use std::time::Instant;

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Effect, EffectKind, Glass, NodeId, NodeKind};
use figcraft_geom::Affine;

use crate::{Options, Retained, View, Work, render_view, render_view_single};

const RUNS: u32 = 8;

/// Stripes, a card under the pane and the glass pane: (document, page, pane, card).
fn scene() -> (Document, NodeId, NodeId, NodeId) {
    let mut d = Document::new("glass bench");
    let p = d.first_page().unwrap();
    let colours = ["E03030", "2040E0", "20C060", "F0C830"].map(|h| Color::from_hex(h).unwrap());
    for i in 0..24u32 {
        let s = d.create(p, NodeKind::Rectangle, "Stripe", f64::from(i) * 50.0, 0.0, 50.0, 800.0).unwrap();
        d.get_mut(s).unwrap().fills = vec![Paint::solid(colours[i as usize % 4])];
    }
    let card = d.create(p, NodeKind::Rectangle, "Card", 420.0, 300.0, 160.0, 120.0).unwrap();
    d.get_mut(card).unwrap().fills = vec![Paint::solid(Color::WHITE)];
    let pane = d.create(p, NodeKind::Rectangle, "Glass", 0.0, 0.0, 600.0, 360.0).unwrap();
    let n = d.get_mut(pane).unwrap();
    n.transform = Affine::translate((300.0, 220.0));
    n.corners.radii = [40.0; 4];
    n.fills = vec![Paint::solid(Color { a: 0.1, ..Color::WHITE })];
    n.effects = vec![Effect { kind: EffectKind::Glass(Glass::default()), visible: true }];
    (d, p, pane, card)
}

fn view(origin: (f64, f64), scale: f64) -> View {
    View { origin, scale, width: 2400, height: 1600 }
}

/// Mean milliseconds of `RUNS` whole-view renders (`render` of the document and view `step(i)`
/// leaves) after one warm-up.
fn time(
    d: &mut Document,
    p: NodeId,
    render: fn(&Document, NodeId, &View, &Options) -> Option<tiny_skia::Pixmap>,
    mut step: impl FnMut(&mut Document, u32) -> View,
) -> f64 {
    let v = step(d, 0);
    let _ = render(d, p, &v, &Options::default());
    let t = Instant::now();
    for i in 1..=RUNS {
        let v = step(d, i);
        assert!(render(d, p, &v, &Options::default()).is_some());
    }
    t.elapsed().as_secs_f64() * 1000.0 / f64::from(RUNS)
}

/// Mean milliseconds of `RUNS` canvas frames at `scale`, each after `edit(document, i)`.
fn canvas(d: &Document, p: NodeId, scale: f64, edit: impl Fn(&mut Document, u32)) -> f64 {
    let mut r = Retained::default();
    let mut doc = Arc::new(d.clone());
    let v = view((0.0, 0.0), scale);
    let _ = r.render(&doc, p, &v, &Options::default());
    let t = Instant::now();
    for i in 1..=RUNS {
        let mut next = (*doc).clone();
        edit(&mut next, i);
        doc = Arc::new(next);
        let (_, _, work) = r.render(&doc, p, &v, &Options::default()).unwrap();
        assert!(matches!(work, Work::Partial { .. }), "{work:?}");
    }
    t.elapsed().as_secs_f64() * 1000.0 / f64::from(RUNS)
}

fn set_glass(d: &mut Document, pane: NodeId, g: Glass) {
    d.get_mut(pane).unwrap().effects = vec![Effect { kind: EffectKind::Glass(g), visible: true }];
}

/// Writes the bench scene's render (rendered twice, so a kept shape is read) to the PNG named by
/// `FIGCRAFT_GLASS_PNG`, to compare builds by eye.
#[test]
#[ignore = "writes a PNG: FIGCRAFT_GLASS_PNG=<path>"]
fn glass_bench_png() {
    let Ok(path) = std::env::var("FIGCRAFT_GLASS_PNG") else { return };
    let (d, p, _, _) = scene();
    let _ = render_view(&d, p, &view((0.0, 0.0), 2.0), &Options::default());
    let pix = render_view(&d, p, &view((0.0, 0.0), 2.0), &Options::default()).unwrap();
    pix.save_png(path).unwrap();
}

#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn glass_render_timings() {
    let (mut d, p, pane, card) = scene();
    let move_card = move |d: &mut Document, i: u32| d.get_mut(card).unwrap().transform = Affine::translate((420.0 + 4.0 * f64::from(i), 300.0));
    let move_pane = move |d: &mut Document, i: u32| d.get_mut(pane).unwrap().transform = Affine::translate((300.0 + 3.0 * f64::from(i), 220.0));
    let rows: Vec<(&str, f64)> = vec![
        ("export: same view again", time(&mut d, p, render_view, |_, _| view((0.0, 0.0), 2.0))),
        (
            "export: move the card under the glass",
            time(&mut d, p, render_view, |d, i| {
                move_card(d, i);
                view((0.0, 0.0), 2.0)
            }),
        ),
        ("export: zoom, a new scale every frame", time(&mut d, p, render_view, |_, i| view((0.0, 0.0), 2.0 + 0.013 * f64::from(i)))),
        (
            "export: zoomed in, a corner of the pane in view, a new scale every frame",
            time(&mut d, p, render_view, |_, i| view((700.0, 400.0), 3.0 + 0.013 * f64::from(i))),
        ),
        ("single: same view again", time(&mut d, p, render_view_single, |_, _| view((0.0, 0.0), 2.0))),
        ("canvas: drag the card under the glass", canvas(&d, p, 2.0, move_card)),
        (
            "canvas: drag a glass slider (light angle)",
            canvas(&d, p, 2.0, |d, i| set_glass(d, pane, Glass { light_angle: -45.0 + f64::from(i), ..Glass::default() })),
        ),
        ("canvas: drag the pane (zoom 200%)", canvas(&d, p, 2.0, move_pane)),
        ("canvas: drag the pane (zoom 137%)", canvas(&d, p, 1.37, move_pane)),
        (
            "canvas: resize the pane",
            canvas(&d, p, 2.0, |d, i| {
                let n = d.get_mut(pane).unwrap();
                n.width = 600.0 + 3.0 * f64::from(i);
            }),
        ),
    ];
    d.get_mut(pane).unwrap().effects.iter_mut().for_each(|e| e.visible = false);
    let without = time(&mut d, p, render_view, |_, _| view((0.0, 0.0), 2.0));
    println!("glass render timings ({} build, 2400 × 1600 px, mean of {RUNS}, ms)", if cfg!(debug_assertions) { "debug" } else { "release" });
    println!("| render | ms |\n|---|---:|");
    for (name, ms) in rows {
        println!("| {name} | {ms:.1} |");
    }
    println!("| export: same view, glass hidden | {without:.1} |");
}
