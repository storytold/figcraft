//! The board canvas's dot grid: on the page colour, under the layers, the same pixels whichever
//! part of the view a render covers (bands, retained patches), never in exports, and nothing
//! drawn (or panicking) for spacings and sizes it can't draw.

use std::sync::Arc;

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, NodeKind};

use crate::{Dots, Options, Retained, View, render_view, render_view_single};

fn dots() -> Dots {
    Dots { step: 24.0, size: 2.0, color: Color::new(0.0, 0.0, 0.0, 0.5), faded: Color::new(0.0, 0.0, 0.0, 0.25), lines: false, major: 2 }
}

fn page_with_square() -> (Document, figcraft_doc::NodeId) {
    let mut d = Document::new("dots");
    let page = d.first_page().unwrap();
    let r = d.create(page, NodeKind::Rectangle, "Square", 40.0, 40.0, 60.0, 60.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::solid(Color::from_hex("2D6CDF").unwrap())];
    (d, page)
}

fn rgba(p: &crate::Pixmap, x: u32, y: u32) -> [u8; 4] {
    let c = p.pixel(x, y).unwrap();
    [c.red(), c.green(), c.blue(), c.alpha()]
}

#[test]
fn dots_sit_on_the_page_under_the_layers() {
    let (d, page) = page_with_square();
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: 200, height: 200 };
    let pix = render_view_single(&d, page, &view, &Options { dots: Some(dots()), ..Default::default() }).unwrap();
    let plain = render_view_single(&d, page, &view, &Options::default()).unwrap();
    // A strong dot at (0, 0) (every second line both ways), a fainter one at (24, 0).
    let (strong, faint, empty) = (rgba(&pix, 0, 0), rgba(&pix, 24, 0), rgba(&pix, 12, 0));
    assert_eq!(empty, rgba(&plain, 12, 0), "nothing between the dots");
    assert!(strong[0] < faint[0] && faint[0] < empty[0], "{strong:?} {faint:?} {empty:?}");
    assert_eq!(rgba(&pix, 48, 48), rgba(&plain, 48, 48), "the square covers the dot at (48, 48)");
    // Exports and thumbnails (no dots asked for) are unchanged.
    assert_ne!(pix.data(), plain.data());
}

#[test]
fn dots_line_up_across_bands_and_patches() {
    let (d, page) = page_with_square();
    let opts = Options { dots: Some(dots()), ..Default::default() };
    // On a whole-pixel origin, as the canvas renders.
    let full = View { origin: (-212.0, -140.0), scale: 1.5, width: 900, height: 700 };
    let a = render_view_single(&d, page, &full, &opts).unwrap();
    // Banded in parallel (desktop) or in one piece: the same pixels.
    let b = render_view(&d, page, &full, &opts).unwrap();
    assert_eq!(a.data(), b.data());
    // A view on the same pixel grid, a whole number of pixels further on, shows the same dots.
    let shifted = View { origin: (full.origin.0 + 101.0 / 1.5, full.origin.1 + 37.0 / 1.5), ..full.clone() };
    let c = render_view_single(&d, page, &shifted, &opts).unwrap();
    for (x, y) in [(0, 0), (150, 90), (400, 300), (700, 500)] {
        assert_eq!(rgba(&c, x, y), rgba(&a, x + 101, y + 37), "at {x}, {y}");
    }
    // An edit redraws only around the layer, and the patched image matches a fresh render.
    let mut retained = Retained::default();
    let doc = Arc::new(d.clone());
    retained.render(&doc, page, &full, &opts).unwrap();
    let mut e = d;
    let sq = e.get(page).unwrap().children[0];
    e.get_mut(sq).unwrap().fills = vec![Paint::solid(Color::from_hex("E8505B").unwrap())];
    let e = Arc::new(e);
    let (snapped, patched, _) = retained.render(&e, page, &full, &opts).unwrap();
    let fresh = render_view_single(&e, page, &snapped, &opts).unwrap();
    assert_eq!(patched.data(), fresh.data());
}

#[test]
fn dots_it_cant_draw_draw_nothing() {
    let (d, page) = page_with_square();
    let view = View { origin: (-1e12, 1e12), scale: 1.0, width: 64, height: 64 };
    let plain = render_view_single(&d, page, &view, &Options::default()).unwrap();
    for step in [0.0, -24.0, f64::NAN, f64::INFINITY, 1e-9] {
        let opts = Options { dots: Some(Dots { step, ..dots() }), ..Default::default() };
        assert_eq!(render_view_single(&d, page, &view, &opts).unwrap().data(), plain.data(), "step {step}");
    }
    for size in [f64::NAN, 1e308, -1.0] {
        let opts = Options { dots: Some(Dots { size, ..dots() }), ..Default::default() };
        let _ = render_view_single(&d, page, &view, &opts);
    }
    let far = View { origin: (f64::MAX / 2.0, -f64::MAX / 2.0), scale: 1e10, width: 16, height: 16 };
    let _ = render_view_single(&d, page, &far, &Options { dots: Some(dots()), ..Default::default() });
    for major in [0, 1, u32::MAX] {
        for lines in [false, true] {
            let opts = Options { dots: Some(Dots { major, lines, ..dots() }), ..Default::default() };
            let _ = render_view_single(&d, page, &view, &opts);
            let _ = render_view_single(&d, page, &far, &opts);
        }
    }
}

#[test]
fn lines_run_both_ways_with_crossings_blended_once() {
    let (d, page) = page_with_square();
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: 200, height: 200 };
    let lines = Dots { lines: true, size: 1.0, ..dots() };
    let pix = render_view_single(&d, page, &view, &Options { dots: Some(lines), ..Default::default() }).unwrap();
    let plain = render_view_single(&d, page, &view, &Options::default()).unwrap();
    // Line 0 is strong, line 1 (at 24) faded, line 2 (at 48) strong again; between them, the page.
    let (strong, faded, empty) = (rgba(&pix, 0, 12), rgba(&pix, 24, 12), rgba(&pix, 12, 12));
    assert_eq!(empty, rgba(&plain, 12, 12));
    assert!(strong[0] < faded[0] && faded[0] < empty[0], "{strong:?} {faded:?} {empty:?}");
    assert_eq!(rgba(&pix, 12, 0), strong, "a horizontal line as strong as a vertical one");
    // A crossing is no darker than the stronger of its lines.
    assert_eq!(rgba(&pix, 0, 0), strong);
    assert_eq!(rgba(&pix, 24, 0), strong, "a faded line crossing a strong one");
    assert_eq!(rgba(&pix, 24, 24), faded);
    // The square covers the lines under it.
    assert_eq!(rgba(&pix, 72, 72), rgba(&plain, 72, 72));
    // Banded or not, the same pixels.
    let wide = View { origin: (-212.0, -140.0), scale: 1.5, width: 900, height: 700 };
    let opts = Options { dots: Some(lines), ..Default::default() };
    assert_eq!(render_view_single(&d, page, &wide, &opts).unwrap().data(), render_view(&d, page, &wide, &opts).unwrap().data());
}

#[test]
fn graph_paper_is_strong_every_major_line() {
    let (d, page) = page_with_square();
    let view = View { origin: (-200.0, -200.0), scale: 1.0, width: 160, height: 160 };
    let graph = Dots { lines: true, size: 1.0, major: 5, step: 10.0, ..dots() };
    let pix = render_view_single(&d, page, &view, &Options { dots: Some(graph), ..Default::default() }).unwrap();
    let at = |x: i32, y: i32| rgba(&pix, (x + 200) as u32, (y + 200) as u32);
    let strong = at(-150, -195);
    for x in [-190, -180, -170, -160, -140, -130, -120, -110] {
        assert_ne!(at(x, -195), strong, "a faded line at {x}");
        assert_eq!(at(x, -195), at(-190, -195));
    }
    for x in [-200, -100, -50] {
        assert_eq!(at(x, -195), strong, "a strong line at {x}");
    }
}
