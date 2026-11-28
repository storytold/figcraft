//! Navigation timings in a large file: five pages of 20,000 layers each (100,000 in the file), seen
//! through a 1600 × 1100 device-pixel view. Run in release:
//! `cargo test --release -p figcraft-render --lib large_doc_timings -- --ignored --nocapture`.
//!
//! Columns, by what the pages hold:
//! - "cards": 400 cards a page (frames with a drop shadow), each holding 49 rectangles;
//! - "blur here": the cards, and a background blur and a glass pane on the page shown;
//! - "blur elsewhere": the cards, and a background blur and a glass pane on another page;
//! - "flat": 20,000 rectangles straight on each page.
//!
//! Rows go through `Retained` as the canvas does: "scan" is what the retained renderer works out
//! about a page after an edit (the layers to redraw whole, the largest backdrop blur); "edit"
//! moves one rectangle each frame; "zoom" renders a new scale each frame; "pan" moves the view by
//! 60 device pixels each frame.

use std::sync::Arc;
use std::time::Instant;

use figcraft_color::{Color, Paint};
use figcraft_doc::{Blur, Document, Effect, EffectKind, Glass, NodeId, NodeKind, Shadow};
use figcraft_geom::Affine;

use crate::{Options, Retained, View, Work};

const PAGES: u32 = 5;
const RUNS: u32 = 12;

#[derive(Clone, Copy, PartialEq)]
enum Scene {
    Cards,
    BlurHere,
    BlurElsewhere,
    Flat,
}

/// The file, the page shown and a rectangle on it.
struct File {
    doc: Document,
    page: NodeId,
    target: NodeId,
}

/// A background blur and a glass pane on `page`.
fn backdrops(d: &mut Document, page: NodeId) {
    let pane = d.create(page, NodeKind::Rectangle, "Frosted", 1100.0, 1100.0, 500.0, 300.0).unwrap();
    let n = d.get_mut(pane).unwrap();
    n.fills = vec![Paint::solid(Color { a: 0.2, ..Color::WHITE })];
    n.effects = vec![Effect { kind: EffectKind::BackgroundBlur(Blur { radius: 24.0, ..Blur::default() }), visible: true }];
    let glass = d.create(page, NodeKind::Rectangle, "Glass", 1700.0, 1300.0, 400.0, 260.0).unwrap();
    let n = d.get_mut(glass).unwrap();
    n.corners.radii = [32.0; 4];
    n.fills = vec![Paint::solid(Color { a: 0.1, ..Color::WHITE })];
    n.effects = vec![Effect { kind: EffectKind::Glass(Glass::default()), visible: true }];
}

fn file(scene: Scene) -> File {
    let mut d = Document::new("large file");
    let first = d.first_page().unwrap();
    let pages: Vec<NodeId> = std::iter::once(first).chain((1..PAGES).map(|i| d.add_page(&format!("Page {}", i + 1)))).collect();
    let mut target = None;
    let colour = |x: u32, y: u32, n: u32| Paint::solid(Color::new(x as f32 / n as f32, y as f32 / n as f32, 0.6, 1.0));
    for &p in &pages {
        if scene == Scene::Flat {
            for y in 0..100u32 {
                for x in 0..200u32 {
                    let r = d.create(p, NodeKind::Rectangle, "Cell", f64::from(x) * 30.0, f64::from(y) * 30.0, 26.0, 26.0).unwrap();
                    d.get_mut(r).unwrap().fills = vec![colour(x, y, 200)];
                    if p == first && x == 50 && y == 50 {
                        target = Some(r);
                    }
                }
            }
            continue;
        }
        for cy in 0..20u32 {
            for cx in 0..20u32 {
                let card = d.create(p, NodeKind::Frame, "Card", f64::from(cx) * 400.0, f64::from(cy) * 400.0, 360.0, 360.0).unwrap();
                let n = d.get_mut(card).unwrap();
                n.fills = vec![Paint::solid(Color::WHITE)];
                n.effects =
                    vec![Effect { kind: EffectKind::DropShadow(Shadow { radius: 12.0, offset: (0.0, 4.0), ..Shadow::default() }), visible: true }];
                for y in 0..7u32 {
                    for x in 0..7u32 {
                        let r =
                            d.create(card, NodeKind::Rectangle, "Cell", 8.0 + f64::from(x) * 50.0, 8.0 + f64::from(y) * 50.0, 44.0, 44.0).unwrap();
                        d.get_mut(r).unwrap().fills = vec![colour(x, y, 7)];
                        if p == first && cx == 3 && cy == 3 && x == 3 && y == 3 {
                            target = Some(r);
                        }
                    }
                }
            }
        }
    }
    match scene {
        Scene::BlurHere => backdrops(&mut d, first),
        Scene::BlurElsewhere => backdrops(&mut d, pages[2]),
        Scene::Cards | Scene::Flat => {}
    }
    File { doc: d, page: first, target: target.unwrap() }
}

fn view(origin: (f64, f64), scale: f64) -> View {
    View { origin, scale, width: 1600, height: 1100 }
}

fn opts() -> Options {
    Options { lod: true, ..Options::default() }
}

fn mean_ms(t: Instant, runs: u32) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0 / f64::from(runs)
}

/// Move the target a unit right.
fn nudge(d: &mut Document, target: NodeId) {
    let n = d.get_mut(target).unwrap();
    let c = n.transform.as_coeffs();
    n.transform = Affine::translate((c[4] + 1.0, c[5]));
}

/// Mean milliseconds of the retained renderer's look at the page after an edit.
fn scan(f: &File) -> f64 {
    let mut doc = f.doc.clone();
    let t = Instant::now();
    for _ in 0..RUNS {
        nudge(&mut doc, f.target);
        std::hint::black_box(crate::retained::bench_scan(&doc, f.page));
    }
    mean_ms(t, RUNS)
}

/// Mean milliseconds of a canvas frame after moving the target a unit.
fn edits(f: &File) -> f64 {
    let v = view((1000.0, 1000.0), 1.0);
    let mut r = Retained::default();
    let mut doc = Arc::new(f.doc.clone());
    r.render(&doc, f.page, &v, &opts()).unwrap();
    let t = Instant::now();
    for _ in 0..RUNS {
        let mut next = (*doc).clone();
        nudge(&mut next, f.target);
        doc = Arc::new(next);
        let (_, _, work) = r.render(&doc, f.page, &v, &opts()).unwrap();
        assert!(matches!(work, Work::Partial { .. }), "{work:?}");
    }
    mean_ms(t, RUNS)
}

/// Mean milliseconds of a canvas frame at a new zoom around `scale`.
fn zooms(f: &File, scale: f64) -> f64 {
    let doc = Arc::new(f.doc.clone());
    let mut r = Retained::default();
    r.render(&doc, f.page, &view((1000.0, 1000.0), scale), &opts()).unwrap();
    let t = Instant::now();
    for i in 1..=RUNS {
        let (_, _, work) = r.render(&doc, f.page, &view((1000.0, 1000.0), scale * (1.0 + 0.02 * f64::from(i))), &opts()).unwrap();
        assert_eq!(work, Work::Full);
    }
    mean_ms(t, RUNS)
}

/// Mean milliseconds of a canvas frame after a pan of 60 device pixels.
fn pans(f: &File) -> f64 {
    let doc = Arc::new(f.doc.clone());
    let mut r = Retained::default();
    let mut at = (1000.0, 1000.0);
    r.render(&doc, f.page, &view(at, 1.0), &opts()).unwrap();
    let t = Instant::now();
    for _ in 0..RUNS {
        at.0 += 60.0;
        let (_, _, work) = r.render(&doc, f.page, &view(at, 1.0), &opts()).unwrap();
        assert!(matches!(work, Work::Partial { .. }), "{work:?}");
    }
    mean_ms(t, RUNS)
}

#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn large_doc_timings() {
    let scenes = [Scene::Cards, Scene::BlurHere, Scene::BlurElsewhere, Scene::Flat].map(file);
    println!(
        "large file timings ({} build, {} layers, 1600 × 1100 px, mean of {RUNS}, ms)",
        if cfg!(debug_assertions) { "debug" } else { "release" },
        scenes[0].doc.node_count()
    );
    println!("| frame | cards | blur here | blur elsewhere | flat |\n|---|---:|---:|---:|---:|");
    let row = |name: &str, f: &dyn Fn(&File) -> f64| {
        let cells: Vec<String> = scenes.iter().map(|s| format!("{:.2}", f(s))).collect();
        println!("| {name} | {} |", cells.join(" | "));
    };
    row("scan after an edit", &scan);
    row("edit one layer", &edits);
    row("zoom (100%)", &|f| zooms(f, 1.0));
    row("zoom (400%)", &|f| zooms(f, 4.0));
    row("pan 60 px", &pans);
}

/// The cards with, on the page shown, a grainy card, a layer filled with a pattern whose source
/// lies on another page, a background blur and a glass pane.
fn mixed() -> File {
    let mut f = file(Scene::BlurHere);
    let d = &mut f.doc;
    let other = d.pages[3];
    let source = d.create(other, NodeKind::Frame, "Source", 0.0, 0.0, 30.0, 30.0).unwrap();
    d.get_mut(source).unwrap().fills = vec![Paint::solid(Color::from_hex("E03030").unwrap())];
    let dot = d.create(source, NodeKind::Rectangle, "Dot", 8.0, 8.0, 12.0, 12.0).unwrap();
    d.get_mut(dot).unwrap().fills = vec![Paint::solid(Color::from_hex("2040E0").unwrap())];
    let filled = d.create(f.page, NodeKind::Rectangle, "Filled", 1300.0, 1500.0, 300.0, 200.0).unwrap();
    let pt = figcraft_color::PatternPaint::new(source.0);
    d.get_mut(filled).unwrap().fills = vec![Paint { kind: figcraft_color::PaintKind::Pattern(pt), ..Paint::solid(Color::BLACK) }];
    let grainy = d.create(f.page, NodeKind::Rectangle, "Grain", 1500.0, 1150.0, 200.0, 120.0).unwrap();
    let n = d.get_mut(grainy).unwrap();
    n.fills = vec![Paint::solid(Color::from_hex("20C060").unwrap())];
    n.effects = vec![Effect {
        kind: EffectKind::Noise(figcraft_doc::Noise { size: 0.5, density: 0.4, color: Color::BLACK, ..Default::default() }),
        visible: true,
    }];
    f
}

/// Prints a digest of every frame of a run of pans, edits and zooms through `Retained`, to tell
/// that two builds draw the same pixels: run in release on each and compare.
#[test]
#[ignore = "digests: run in release with --ignored --nocapture on two builds and compare"]
fn large_doc_digests() {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let scenes = [
        ("cards", file(Scene::Cards)),
        ("blur here", file(Scene::BlurHere)),
        ("blur elsewhere", file(Scene::BlurElsewhere)),
        ("flat", file(Scene::Flat)),
        ("mixed", mixed()),
    ];
    let mut all = DefaultHasher::new();
    for (name, f) in &scenes {
        let mut r = Retained::default();
        let mut doc = Arc::new(f.doc.clone());
        let mut frames = Vec::new();
        let mut at = (1000.0, 1000.0);
        let mut scale = 1.0;
        let steps = ["pan", "pan", "edit", "edit", "zoom 1.37", "pan", "edit", "zoom 4", "pan", "edit", "zoom 0.5", "pan"];
        let mut draw = |r: &mut Retained, doc: &Arc<Document>, at: (f64, f64), scale: f64, frames: &mut Vec<String>| {
            let (_, pix, work) = r.render(doc, f.page, &view(at, scale), &opts()).unwrap();
            let mut h = DefaultHasher::new();
            pix.data().hash(&mut h);
            let w = match work {
                Work::Full => "F",
                Work::Partial { .. } => "P",
                Work::Reused => "R",
            };
            frames.push(format!("{w}{:04x}", h.finish() & 0xffff));
            pix.data().hash(&mut all);
        };
        draw(&mut r, &doc, at, scale, &mut frames);
        for s in steps {
            match s {
                "pan" => at = (at.0 + 60.0 / scale, at.1 + 24.0 / scale),
                "edit" => {
                    let mut next = (*doc).clone();
                    nudge(&mut next, f.target);
                    doc = Arc::new(next);
                }
                z => scale = z.trim_start_matches("zoom ").parse().unwrap(),
            }
            draw(&mut r, &doc, at, scale, &mut frames);
        }
        println!("{name}: {}", frames.join(" "));
    }
    println!("all: {:016x}", all.finish());
}
