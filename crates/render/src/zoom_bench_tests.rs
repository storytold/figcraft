//! Timings of a shadowed card seen close up: a small shape inside a rounded picture inside a
//! rounded card with a drop shadow, inside two stacks and a screen, at 1,933% on a 2× display
//! (38.66 device pixels a unit, 4112 × 2292 device pixels). The mobile app template's trail card,
//! where the browser took 16.6 s for one render and 2 s for each frame of an edit.
//!
//! Run in release, on one thread as the browser does:
//! `RAYON_NUM_THREADS=1 cargo test --release -p figcraft-render --lib zoomed_card_timings -- --ignored --nocapture`.

use std::sync::Arc;
use std::time::Instant;

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Effect, EffectKind, NodeId, NodeKind, Shadow};

use crate::{Begun, Options, Retained, View, Work, render_view_single};

/// Device pixels a document unit: 1,933% at a pixel ratio of 2.
pub(crate) const SCALE: f64 = 38.66;

pub(crate) struct Card {
    pub doc: Document,
    pub page: NodeId,
    pub card: NodeId,
    /// The shape in the card's picture.
    pub shape: NodeId,
}

/// The card: screen (clipping) > content > list > card (rounded, white, a drop shadow; like
/// the two stacks around it, not clipping) > picture (rounded, clipping) > shape.
pub(crate) fn card(shadow: bool) -> Card {
    let fill = |d: &mut Document, id: NodeId, hex: &str| d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex(hex).unwrap())];
    let mut d = Document::new("zoomed card");
    let page = d.first_page().unwrap();
    let screen = d.create(page, NodeKind::Frame, "Screen", 470.0, 0.0, 390.0, 844.0).unwrap();
    fill(&mut d, screen, "F4F6F5");
    let content = d.create(screen, NodeKind::Frame, "Content", 0.0, 81.0, 390.0, 763.0).unwrap();
    let list = d.create(content, NodeKind::Frame, "List", 24.0, 180.0, 342.0, 312.0).unwrap();
    let card = d.create(list, NodeKind::Frame, "Card", 0.0, 108.0, 342.0, 96.0).unwrap();
    fill(&mut d, card, "FFFFFF");
    let n = d.get_mut(card).unwrap();
    n.corners.radii = [20.0; 4];
    if shadow {
        n.effects.push(Effect {
            kind: EffectKind::DropShadow(Shadow { offset: (0.0, 4.0), radius: 16.0, color: Color { a: 0.08, ..Color::BLACK }, ..Shadow::default() }),
            visible: true,
        });
    }
    for stack in [content, list, card] {
        d.get_mut(stack).unwrap().clip = false;
    }
    let picture = d.create(card, NodeKind::Frame, "Picture", 12.0, 12.0, 72.0, 72.0).unwrap();
    fill(&mut d, picture, "CFE3DA");
    d.get_mut(picture).unwrap().corners.radii = [14.0; 4];
    let shape = d.create(picture, NodeKind::Polygon { count: 5 }, "Shape", 0.0, 23.0, 72.0, 49.0).unwrap();
    fill(&mut d, shape, "7FAE9C");
    Card { doc: d, page, card, shape }
}

/// The view the canvas has after zooming to the shape: the shape in the middle.
pub(crate) fn close_up(c: &Card, width: u32, height: u32) -> View {
    let b = crate::draw::paint_bounds(&c.doc, c.shape).unwrap();
    let (cx, cy) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
    View { origin: (cx - f64::from(width) / SCALE / 2.0, cy - f64::from(height) / SCALE / 2.0), scale: SCALE, width, height }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// A full render drawn in the browser's bands: (all of it, the slowest band), in milliseconds.
fn banded(c: &Card, v: &View) -> (f64, f64) {
    let doc = Arc::new(c.doc.clone());
    let mut r = Retained::default();
    let band_h = (v.height + 1).div_ceil(8).max(64);
    let t = Instant::now();
    let mut slowest = 0.0f64;
    match r.begin(&doc, c.page, v, &Options::default(), band_h).unwrap() {
        Begun::Done(..) => {}
        Begun::Steps(mut steps) => loop {
            let b = Instant::now();
            let done = steps.step();
            slowest = slowest.max(ms(b));
            if done {
                break;
            }
        },
    }
    (ms(t), slowest)
}

/// Mean milliseconds of a canvas frame after an edit of the shape, and what the last one took.
fn edits(c: &Card, v: &View) -> (f64, Work) {
    const RUNS: u32 = 6;
    let mut r = Retained::default();
    let mut doc = Arc::new(c.doc.clone());
    let _ = r.render(&doc, c.page, v, &Options::default());
    let mut work = Work::Reused;
    let t = Instant::now();
    for i in 1..=RUNS {
        let mut next = (*doc).clone();
        next.get_mut(c.shape).unwrap().height = 49.0 - f64::from(i);
        doc = Arc::new(next);
        work = r.render(&doc, c.page, v, &Options::default()).unwrap().2;
    }
    (ms(t) / f64::from(RUNS), work)
}

#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn zoomed_card_timings() {
    println!("zoomed card timings ({} build, 4112 × 2292 px, ms)", if cfg!(debug_assertions) { "debug" } else { "release" });
    println!("| render | shadow | no shadow |\n|---|---:|---:|");
    let (with, without) = (card(true), card(false));
    let v = close_up(&with, 4112, 2292);
    let piece = |c: &Card| {
        let t = Instant::now();
        assert!(render_view_single(&c.doc, c.page, &v, &Options::default()).is_some());
        ms(t)
    };
    println!("| the view in one piece | {:.0} | {:.0} |", piece(&with), piece(&without));
    let (a, b) = (banded(&with, &v), banded(&without, &v));
    println!("| the view in 8 bands | {:.0} | {:.0} |", a.0, b.0);
    println!("| the slowest band | {:.0} | {:.0} |", a.1, b.1);
    let (a, b) = (edits(&with, &v), edits(&without, &v));
    println!("| a frame after an edit of the shape | {:.0} ({:?}) | {:.0} ({:?}) |", a.0, a.1, b.0, b.1);
}
