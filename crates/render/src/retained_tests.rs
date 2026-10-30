//! The retained renderer: every partial render must match a full render of the same view.

use std::sync::{Arc, Once};

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Effect, EffectKind, Node, NodeId, NodeKind, Shadow, TextAutoResize, TextData};
use figcraft_geom::Affine;

use crate::{Options, Pixmap, Retained, View, Work, render_view, retained::snap};

/// System fonts load once, before any render here, so text has glyphs and fonts never change
/// mid-test (a font change makes the next render full).
fn fonts() {
    static ONCE: Once = Once::new();
    ONCE.call_once(figcraft_text::scan_system_fonts);
}

struct Scene {
    doc: Arc<Document>,
    page: NodeId,
    rect: NodeId,
    text: NodeId,
    frame: NodeId,
    child: NodeId,
}

fn solid(n: &mut Node, hex: &str) {
    n.fills = vec![Paint::solid(Color::from_hex(hex).unwrap())];
}

/// Shapes, a clipping frame with children, a group with a drop shadow and fixed-width text; with
/// `cut_sensitive`, also layers whose look depends on where a render cuts them (inner shadow and
/// noise, a background blur). A full render cuts those at the view's edge, so tests compare them
/// only in views that contain them whole.
fn scene(cut_sensitive: bool) -> Scene {
    fonts();
    let mut d = Document::new("retained");
    let page = d.first_page().unwrap();
    let rect = d.create(page, NodeKind::Rectangle, "R", 20.0, 20.0, 80.0, 50.0).unwrap();
    solid(d.get_mut(rect).unwrap(), "E03030");
    let ell = d.create(page, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "E", 140.0, 30.0, 60.0, 60.0).unwrap();
    solid(d.get_mut(ell).unwrap(), "3070E0");
    let frame = d.create(page, NodeKind::Frame, "F", 40.0, 120.0, 200.0, 120.0).unwrap();
    let child = d.create(frame, NodeKind::Rectangle, "C", 150.0, 20.0, 90.0, 40.0).unwrap();
    solid(d.get_mut(child).unwrap(), "20A060");
    let group = d.create(page, NodeKind::Group, "G", 280.0, 40.0, 60.0, 60.0).unwrap();
    let g1 = d.create(group, NodeKind::Rectangle, "G1", 0.0, 0.0, 60.0, 60.0).unwrap();
    solid(d.get_mut(g1).unwrap(), "F0B020");
    d.get_mut(group)
        .unwrap()
        .effects
        .push(Effect { kind: EffectKind::DropShadow(Shadow { offset: (12.0, 16.0), radius: 6.0, ..Shadow::default() }), visible: true });
    let mut td = TextData::new("A line of text that overflows its box");
    td.auto_resize = TextAutoResize::None;
    td.style.font_size = 14.0;
    let text = d.create(page, NodeKind::Text(td), "T", 20.0, 270.0, 60.0, 20.0).unwrap();
    if cut_sensitive {
        let inner = d.create(page, NodeKind::Rectangle, "Inner", 180.0, 150.0, 70.0, 50.0).unwrap();
        solid(d.get_mut(inner).unwrap(), "FFFFFF");
        d.get_mut(inner).unwrap().effects.push(Effect::inner_shadow());
        d.get_mut(inner).unwrap().effects.push(Effect {
            kind: EffectKind::Noise(figcraft_doc::Noise { size: 1.0, density: 0.3, color: Color::BLACK, ..Default::default() }),
            visible: true,
        });
        let glass = d.create(page, NodeKind::Rectangle, "Glass", 120.0, 280.0, 120.0, 40.0).unwrap();
        d.get_mut(glass).unwrap().fills = vec![Paint::solid(Color::new(1.0, 1.0, 1.0, 0.3))];
        d.get_mut(glass).unwrap().effects.push(Effect { kind: EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(8.0)), visible: true });
    }
    Scene { doc: Arc::new(d), page, rect, text, frame, child }
}

fn view(x: f64, y: f64, scale: f64) -> View {
    View { origin: (x, y), scale, width: 300, height: 220 }
}

fn edit(doc: &Arc<Document>, f: impl FnOnce(&mut Document)) -> Arc<Document> {
    let mut d = (**doc).clone();
    f(&mut d);
    Arc::new(d)
}

/// Render through `r` and check the image against a full render of the same (snapped) view.
fn check(r: &mut Retained, doc: &Arc<Document>, page: NodeId, v: &View) -> Work {
    let (shown, pix, work) = r.render(doc, page, v, &Options::default()).unwrap();
    let pix = pix.clone();
    let key = |v: &View| (v.origin, v.scale, v.width, v.height);
    if let Some((snapped, _)) = snap(v) {
        assert_eq!(key(&shown), key(&snapped), "the snapped view is what's shown");
    }
    let full = render_view(doc, page, &shown, &Options::default()).unwrap();
    assert_same(&pix, &full, &format!("{work:?}"));
    work
}

/// Equal up to anti-aliasing: a part rendered on its own sees its paths translated by a whole
/// number of pixels, and float rounding can move an edge sample (a fraction of one pixel's
/// coverage along a curve). Stale or missing areas differ by whole colours, over many pixels.
pub(crate) fn assert_same(a: &Pixmap, b: &Pixmap, what: &str) {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    let (mut slight, mut big, mut first) = (0usize, 0usize, None);
    for (i, (x, y)) in a.data().iter().zip(b.data()).enumerate() {
        let d = x.abs_diff(*y);
        slight += usize::from(d > 2);
        big += usize::from(d > 96);
        if d > 2 && first.is_none() {
            first = Some(((i / 4) as u32 % a.width(), (i / 4) as u32 / a.width(), d));
        }
    }
    assert!(slight <= 200 && big == 0, "{what}: partial render differs from a full one ({slight} channels over 2, {big} over 96, first {first:?})");
}

fn partial(w: Work) -> bool {
    matches!(w, Work::Partial { .. })
}

#[test]
fn unchanged_view_is_reused() {
    let s = scene(false);
    let mut r = Retained::default();
    assert_eq!(check(&mut r, &s.doc, s.page, &view(0.0, 0.0, 1.0)), Work::Full);
    assert_eq!(check(&mut r, &s.doc, s.page, &view(0.0, 0.0, 1.0)), Work::Reused);
    // A sub-pixel move of the view stays on the same pixel grid.
    assert_eq!(check(&mut r, &s.doc, s.page, &view(0.3, 0.2, 1.0)), Work::Reused);
}

#[test]
fn pans_render_only_the_uncovered_strips() {
    let s = scene(false);
    let mut r = Retained::default();
    check(&mut r, &s.doc, s.page, &view(0.0, 0.0, 1.0));
    for (x, y) in [(37.0, 0.0), (37.0, 23.0), (-12.5, 41.0), (40.0, 80.0), (10.0, 30.0), (0.0, 0.0)] {
        eprintln!("pan to {x} {y}");
        let w = check(&mut r, &s.doc, s.page, &view(x, y, 1.0));
        assert!(partial(w), "pan to ({x}, {y}): {w:?}");
    }
    // At another zoom too (the grid is that zoom's).
    check(&mut r, &s.doc, s.page, &view(10.0, 10.0, 1.7));
    assert!(partial(check(&mut r, &s.doc, s.page, &view(25.0, -8.0, 1.7))));
    // A jump further than the view renders it whole.
    assert_eq!(check(&mut r, &s.doc, s.page, &view(2000.0, 0.0, 1.7)), Work::Full);
}

#[test]
fn edits_render_only_what_changed() {
    let s = scene(false);
    let mut r = Retained::default();
    let v = view(0.0, 0.0, 1.0);
    check(&mut r, &s.doc, s.page, &v);
    let moved = edit(&s.doc, |d| d.get_mut(s.rect).unwrap().transform = Affine::translate((60.0, 40.0)));
    let w = check(&mut r, &moved, s.page, &v);
    let Work::Partial { pixels } = w else { panic!("move: {w:?}") };
    assert!(pixels < 300 * 220 / 4, "only around the layer: {pixels}");
    let recolored = edit(&moved, |d| solid(d.get_mut(s.child).unwrap(), "000000"));
    assert!(partial(check(&mut r, &recolored, s.page, &v)));
    // Out of the clipping frame and back.
    let out = edit(&recolored, |d| d.get_mut(s.child).unwrap().transform = Affine::translate((180.0, 90.0)));
    assert!(partial(check(&mut r, &out, s.page, &v)));
    let hidden = edit(&out, |d| d.get_mut(s.child).unwrap().visible = false);
    assert!(partial(check(&mut r, &hidden, s.page, &v)));
    let removed = edit(&hidden, |d| d.remove(s.child).unwrap());
    assert!(partial(check(&mut r, &removed, s.page, &v)));
    let added = edit(&removed, |d| {
        let k = d.create(s.frame, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "K", 10.0, 10.0, 30.0, 30.0).unwrap();
        solid(d.get_mut(k).unwrap(), "8040C0");
    });
    assert!(partial(check(&mut r, &added, s.page, &v)));
    // Undo: back to an older version of the document.
    assert!(partial(check(&mut r, &s.doc, s.page, &v)));
}

#[test]
fn edits_under_and_beside_effects_match() {
    let s = scene(true);
    let mut r = Retained::default();
    // Holds the inner shadow + noise layer (180, 150) and the background blur (120, 280) whole.
    let v = view(0.0, 100.0, 1.0);
    check(&mut r, &s.doc, s.page, &v);
    // Under the background blur, then across the inner shadow + noise layer.
    let under = edit(&s.doc, |d| d.get_mut(s.rect).unwrap().transform = Affine::translate((150.0, 260.0)));
    assert!(partial(check(&mut r, &under, s.page, &v)));
    // Panning the blur (over the red layer) in from the view's edge, where a full render cut its
    // backdrop off (so only the view after it compares).
    let mut fresh = Retained::default();
    assert_eq!(fresh.render(&under, s.page, &view(0.0, 75.0, 1.0), &Options::default()).map(|r| r.2), Some(Work::Full));
    assert!(partial(check(&mut fresh, &under, s.page, &v)));
    let across = edit(&under, |d| d.get_mut(s.rect).unwrap().transform = Affine::translate((200.0, 170.0)));
    assert!(partial(check(&mut r, &across, s.page, &v)));
    let beside = edit(&across, |d| d.get_mut(s.rect).unwrap().transform = Affine::translate((10.0, 110.0)));
    assert!(partial(check(&mut r, &beside, s.page, &v)));
    // A child of a group with a drop shadow: the shadow moves too, past the group's box with the
    // child.
    let g1 = s.doc.find_by_name("G1", None)[0];
    let v = View { origin: (260.0, 0.0), scale: 1.0, width: 300, height: 220 };
    check(&mut r, &beside, s.page, &v);
    let shadowed = edit(&beside, |d| d.get_mut(g1).unwrap().transform = Affine::translate((30.0, 20.0)));
    assert!(partial(check(&mut r, &shadowed, s.page, &v)));
    let shadowed2 = edit(&shadowed, |d| d.get_mut(g1).unwrap().transform = Affine::translate((10.0, 60.0)));
    assert!(partial(check(&mut r, &shadowed2, s.page, &v)));
}

#[test]
fn text_and_mask_edits_match() {
    let s = scene(false);
    let mut r = Retained::default();
    let v = view(0.0, 120.0, 1.5);
    check(&mut r, &s.doc, s.page, &v);
    // Longer text overflows further than its box.
    let longer = edit(&s.doc, |d| {
        if let NodeKind::Text(t) = &mut d.get_mut(s.text).unwrap().kind {
            t.characters = "Overflowing text, much longer than before, runs on and on".into();
        }
    });
    assert!(partial(check(&mut r, &longer, s.page, &v)));
    // Pans whose new strips hold only the overflowing part of the text.
    for x in [30.0, 60.0, 90.0] {
        assert!(partial(check(&mut r, &longer, s.page, &View { origin: (x, 120.0), ..v.clone() })));
    }
    check(&mut r, &longer, s.page, &v);
    // A mask clips the layers above it, wherever they are.
    let masked = edit(&longer, |d| {
        let m = d.create(s.frame, NodeKind::Rectangle, "M", 0.0, 0.0, 60.0, 60.0).unwrap();
        d.reorder(m, 0).unwrap();
        d.get_mut(m).unwrap().is_mask = true;
    });
    check(&mut r, &masked, s.page, &v);
    let unmasked = edit(&masked, |d| {
        let m = d.find_by_name("M", None)[0];
        d.get_mut(m).unwrap().is_mask = false;
    });
    check(&mut r, &unmasked, s.page, &v);
}

#[test]
fn zoom_page_and_options_render_full() {
    let s = scene(false);
    let mut r = Retained::default();
    let v = view(0.0, 0.0, 1.0);
    check(&mut r, &s.doc, s.page, &v);
    assert_eq!(check(&mut r, &s.doc, s.page, &view(0.0, 0.0, 1.25)), Work::Full);
    // Page background: the whole page changes.
    let bg = edit(&s.doc, |d| {
        if let NodeKind::Page { background, .. } = &mut d.get_mut(s.page).unwrap().kind {
            *background = Color::BLACK;
        }
    });
    assert_eq!(check(&mut r, &bg, s.page, &view(0.0, 0.0, 1.25)), Work::Full);
    // Another page.
    let two = edit(&bg, |d| {
        d.add_page("Two");
    });
    let p2 = two.pages[1];
    check(&mut r, &two, s.page, &view(0.0, 0.0, 1.25));
    assert_eq!(check(&mut r, &two, p2, &view(0.0, 0.0, 1.25)), Work::Full);
    // Outline mode.
    let (_, _, w) = r.render(&two, p2, &view(0.0, 0.0, 1.25), &Options { outline_mode: true, ..Default::default() }).unwrap();
    assert_eq!(w, Work::Full);
}

#[test]
fn junk_views_never_panic() {
    let s = scene(false);
    let mut r = Retained::default();
    for v in [view(f64::NAN, 0.0, 1.0), view(0.0, 0.0, f64::INFINITY), view(1e300, -1e300, 1.0), view(0.0, 0.0, 0.0), view(0.0, 0.0, 1.0)] {
        let _ = r.render(&s.doc, s.page, &v, &Options::default());
    }
    let tiny = View { origin: (0.0, 0.0), scale: 1.0, width: 0, height: 0 };
    assert!(r.render(&s.doc, s.page, &tiny, &Options::default()).is_some());
}

#[test]
fn overflowing_text_is_drawn_while_its_box_is_out_of_view() {
    fonts();
    let mut d = Document::new("overflow");
    let page = d.first_page().unwrap();
    // An auto-width box whose stored width is stale (a file saved where fonts measured narrower):
    // one line, far wider than the box.
    let mut td = TextData::new("Overflowing far to the right of a one pixel box");
    td.auto_resize = TextAutoResize::WidthAndHeight;
    td.style.font_size = 16.0;
    let t = d.create(page, NodeKind::Text(td), "T", 100.0, 10.0, 1.0, 20.0).unwrap();
    let Some((layout, _)) = figcraft_text::layout_node(d.node(t).unwrap()) else { return };
    if layout.glyphs.is_empty() || layout.width < 120.0 {
        return; // no fonts on this machine
    }
    // The box (x 100..101) is left of the view; the glyphs run on into it.
    let v = View { origin: (150.0, 0.0), scale: 1.0, width: 100, height: 40 };
    let pix = render_view(&d, page, &v, &Options { background: Some(Color::WHITE), ..Default::default() }).unwrap();
    let dark = pix.data().as_chunks::<4>().0.iter().filter(|c| c[0] < 128).count();
    assert!(dark > 20, "glyphs drawn: {dark}");
}

/// Draw a full render through [`Retained::begin`] `band_h` rows at a time; how many steps it took.
fn stepped(r: &mut Retained, doc: &Arc<Document>, page: NodeId, v: &View, band_h: u32) -> (View, Pixmap, usize) {
    match r.begin(doc, page, v, &Options::default(), band_h).unwrap() {
        crate::Begun::Done(shown, pix, work) => {
            assert_eq!(work, Work::Full, "one piece only when a blur can't be measured");
            (shown, pix.clone(), 0)
        }
        crate::Begun::Steps(mut steps) => {
            let mut n = 1;
            while !steps.step() {
                n += 1;
                assert!(n < 1000, "steps end");
            }
            let (shown, pix) = r.finish(*steps);
            (shown, pix.clone(), n)
        }
    }
}

#[test]
fn a_render_drawn_band_by_band_matches_one_in_one_piece() {
    for cut_sensitive in [false, true] {
        let s = scene(cut_sensitive);
        // The whole scene in view, so the glass's backdrop is never cut by the view.
        let v = View { origin: (-10.0, -10.0), scale: 1.0, width: 380, height: 360 };
        let mut r = Retained::default();
        let (shown, pix, n) = stepped(&mut r, &s.doc, s.page, &v, 40);
        assert_eq!(n, shown.height.div_ceil(40) as usize, "a band a step");
        let full = render_view(&s.doc, s.page, &shown, &Options::default()).unwrap();
        assert_same(&pix, &full, "bands");
        // Kept as a full render: the next pan patches it.
        assert!(partial(check(&mut r, &s.doc, s.page, &View { origin: (0.0, -10.0), ..v })));
    }
}

#[test]
fn bands_thinner_than_a_blur_reaches_grow_to_twice_its_reach() {
    let s = scene(true);
    let v = View { origin: (-10.0, -10.0), scale: 4.0, width: 380, height: 360 };
    let reach = crate::band_overlap(&s.doc, &v, u32::MAX).unwrap();
    assert!(reach > 4, "the blur reaches past the bands asked for");
    let mut r = Retained::default();
    let (shown, pix, n) = stepped(&mut r, &s.doc, s.page, &v, 4);
    assert_eq!(n, shown.height.div_ceil(4.max(reach.min(shown.height) * 2)) as usize, "bands twice the reach");
    assert_same(&pix, &render_view(&s.doc, s.page, &shown, &Options::default()).unwrap(), "taller bands");
}

#[test]
fn a_reused_or_patched_render_needs_no_steps() {
    let s = scene(false);
    let v = view(0.0, 0.0, 1.0);
    let mut r = Retained::default();
    check(&mut r, &s.doc, s.page, &v);
    assert!(matches!(r.begin(&s.doc, s.page, &v, &Options::default(), 40), Some(crate::Begun::Done(_, _, Work::Reused))));
    let panned = View { origin: (12.0, 0.0), ..v };
    assert!(matches!(r.begin(&s.doc, s.page, &panned, &Options::default(), 40), Some(crate::Begun::Done(_, _, Work::Partial { .. }))));
}

/// Background blurs stacked on each other (the upper reads the lower's blurred pixels) across the
/// band boundaries: the bands' overlap, the reach of the largest blur, is enough for the image to
/// match one drawn in one piece, as it is for the desktop's parallel bands.
#[test]
fn stacked_backdrop_blurs_draw_the_same_in_bands() {
    fonts();
    let mut d = Document::new("stacked");
    let page = d.first_page().unwrap();
    // Stripes to blur, then three translucent blurred panels overlapping each other.
    for i in 0..40 {
        let r = d.create(page, NodeKind::Rectangle, "S", 0.0, f64::from(i) * 10.0, 400.0, 5.0).unwrap();
        solid(d.get_mut(r).unwrap(), if i % 2 == 0 { "E03030" } else { "3070E0" });
    }
    for (x, y) in [(40.0, 60.0), (120.0, 120.0), (60.0, 200.0)] {
        let b = d.create(page, NodeKind::Rectangle, "B", x, y, 220.0, 160.0).unwrap();
        d.get_mut(b).unwrap().fills = vec![Paint::solid(Color::new(1.0, 1.0, 1.0, 0.25))];
        d.get_mut(b).unwrap().effects.push(Effect { kind: EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(24.0)), visible: true });
    }
    let doc = Arc::new(d);
    let v = View { origin: (0.0, 0.0), scale: 1.0, width: 400, height: 400 };
    for band_h in [64, 100, 200] {
        let mut r = Retained::default();
        let (shown, pix, n) = stepped(&mut r, &doc, page, &v, band_h);
        assert!(n >= 3, "{n} bands of {band_h} rows");
        assert_same(&pix, &render_view(&doc, page, &shown, &Options::default()).unwrap(), &format!("bands of {band_h} rows"));
    }
}

/// Layer blurs stacked on one layer apply one after another, so they reach as far as all of them
/// together: a layer cut at a band's edge keeps that much margin, and the bands (the browser's,
/// the desktop's) match a render in one piece, as does a view cut across the blur.
#[test]
fn sequential_layer_blurs_draw_the_same_in_bands() {
    fonts();
    for (count, radius) in [(4, 25.0), (4, 100.0), (8, 10.0)] {
        let mut d = Document::new("sequential");
        let page = d.first_page().unwrap();
        for i in 0..40 {
            let r = d.create(page, NodeKind::Rectangle, "S", 0.0, f64::from(i) * 10.0, 400.0, 5.0).unwrap();
            solid(d.get_mut(r).unwrap(), if i % 2 == 0 { "E03030" } else { "3070E0" });
        }
        let b = d.create(page, NodeKind::Rectangle, "B", 100.0, 150.0, 200.0, 100.0).unwrap();
        solid(d.get_mut(b).unwrap(), "20C040");
        for _ in 0..count {
            d.get_mut(b).unwrap().effects.push(Effect { kind: EffectKind::LayerBlur(figcraft_doc::Blur::uniform(radius)), visible: true });
        }
        let doc = Arc::new(d);
        let what = format!("{count} blurs of {radius}");
        let v = View { origin: (0.0, 0.0), scale: 1.0, width: 400, height: 400 };
        for band_h in [64, 100] {
            let mut r = Retained::default();
            let (shown, pix, n) = stepped(&mut r, &doc, page, &v, band_h);
            assert!(n >= 4, "{what}: {n} bands of {band_h} rows");
            let one = crate::render_view_single(&doc, page, &shown, &Options::default()).unwrap();
            assert_within(&pix, &one, 0, 2, &format!("{what}, bands of {band_h} rows"));
        }
        // The desktop splits a view this large into parallel bands.
        let big = View { origin: (0.0, 0.0), scale: 1.0, width: 800, height: 800 };
        let one = crate::render_view_single(&doc, page, &big, &Options::default()).unwrap();
        assert_within(&render_view(&doc, page, &big, &Options::default()).unwrap(), &one, 0, 2, &format!("{what}, parallel bands"));
        // A view starting halfway down the blur: its rows as in the whole view.
        let cut = View { origin: (0.0, 200.0), scale: 1.0, width: 800, height: 600 };
        let part = crate::render_view_single(&doc, page, &cut, &Options::default()).unwrap();
        assert_within(&part, &one, 200, 2, &format!("{what}, view cut across the blur"));
    }
}

/// A drop shadow is cast from the blurred layer: a layer cut at the view keeps the blur's reach on
/// top of the shadow's, or a rectangle above the view casts a shadow into it from a cut blur.
#[test]
fn a_shadow_cast_from_a_blurred_layer_draws_the_same_in_a_cut_view() {
    fonts();
    let mut d = Document::new("shadowed blur");
    let page = d.first_page().unwrap();
    let b = d.create(page, NodeKind::Rectangle, "B", 100.0, 0.0, 200.0, 100.0).unwrap();
    solid(d.get_mut(b).unwrap(), "20C040");
    let shadow = Shadow { color: Color::new(0.0, 0.0, 0.0, 1.0), offset: (0.0, 300.0), radius: 0.0, ..Shadow::default() };
    d.get_mut(b).unwrap().effects = vec![
        Effect { kind: EffectKind::LayerBlur(figcraft_doc::Blur::uniform(50.0)), visible: true },
        Effect { kind: EffectKind::DropShadow(shadow), visible: true },
    ];
    let whole = View { origin: (0.0, 0.0), scale: 1.0, width: 400, height: 600 };
    let one = crate::render_view_single(&d, page, &whole, &Options::default()).unwrap();
    // The view starts below the rectangle, near the bottom of its shadow: a margin of the shadow's
    // reach alone would cut the layer across the rectangle.
    let cut = View { origin: (0.0, 390.0), scale: 1.0, width: 400, height: 210 };
    let part = crate::render_view_single(&d, page, &cut, &Options::default()).unwrap();
    assert_within(&part, &one, 390, 2, "the shadow of a blurred layer above the view");
}

/// Reaches too large to add (texture of 1e308, twice) each count up to the cap and add up: a margin
/// that stays finite, not none, and not the largest one alone.
#[test]
fn effect_margins_too_large_to_add_stay_large() {
    let mut n = Node::new(NodeId(1), NodeKind::Rectangle, "T", 10.0, 10.0);
    let t = figcraft_doc::Texture { radius: 1e308, ..Default::default() };
    n.effects = vec![Effect { kind: EffectKind::Texture(t.clone()), visible: true }, Effect { kind: EffectKind::Texture(t), visible: true }];
    let m = crate::draw::effect_margin(&n);
    assert_eq!(m, 2.0 * crate::draw::MAX_REACH);
}

#[test]
fn a_moved_layer_renders_where_its_effects_paint_and_no_further() {
    fonts();
    let shadow = |dy: f64, r: f64| Effect {
        kind: EffectKind::DropShadow(Shadow { color: Color::BLACK, offset: (0.0, dy), radius: r, ..Shadow::default() }),
        ..Effect::drop_shadow()
    };
    let blur = Effect { kind: EffectKind::LayerBlur(figcraft_doc::Blur::uniform(30.0)), ..Effect::layer_blur() };
    for (why, effects) in [("a shadow falling 40", vec![shadow(40.0, 8.0)]), ("a blur of 30", vec![blur])] {
        let mut d = Document::new("moved");
        let p = d.first_page().unwrap();
        let id = d.create(p, NodeKind::Rectangle, "R", 300.0, 200.0, 120.0, 120.0).unwrap();
        solid(d.node_mut(id).unwrap(), "3366FF");
        d.node_mut(id).unwrap().effects = effects;
        let before = Arc::new(d);
        let after = edit(&before, |d| d.get_mut(id).unwrap().transform = Affine::translate((315.0, 210.0)));
        let v = View { origin: (0.0, 0.0), scale: 1.0, width: 1000, height: 800 };
        let mut r = Retained::default();
        check(&mut r, &before, p, &v);
        // Where it painted and where it paints now, a pixel round each for rounding out; a quarter
        // more, as layers grow by a shadow's reach on every side, not only towards its offset.
        let area = |doc: &Document| doc.world_render_bounds(id).map_or(0.0, |b| (b.width() + 2.0) * (b.height() + 2.0));
        let most = (area(&before) + area(&after)) * 1.25;
        match check(&mut r, &after, p, &v) {
            Work::Partial { pixels } => assert!(pixels as f64 <= most, "{why}: {pixels} pixels rendered, where it paints is {most}"),
            w => panic!("{why}: {w:?}"),
        }
    }
}

/// Every channel of `a` within `tol` of the same channel of `b` from row `rows_down` on.
fn assert_within(a: &Pixmap, b: &Pixmap, rows_down: u32, tol: u8, what: &str) {
    assert_eq!(a.width(), b.width(), "{what}: widths");
    let row = a.width() as usize * 4;
    let below = b.data().get(rows_down as usize * row..).unwrap();
    let worst = a.data().iter().zip(below).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
    assert!(worst <= tol, "{what}: differs by {worst} levels");
}

#[test]
fn backdrops_at_the_views_edge_patch_as_they_render() {
    fonts();
    let blur = |r: f64| {
        let mut e = Effect::background_blur();
        if let EffectKind::BackgroundBlur(b) = &mut e.kind {
            b.radius = r;
        }
        e
    };
    let mut glass = Effect::glass();
    if let EffectKind::Glass(g) = &mut glass.kind {
        (g.depth, g.refraction) = (30.0, 1.0);
    }
    // A layer reading its backdrop across the view's left edge, over stripes, and a square moved
    // beside it in the view and off it: what the patch reads past the edge a full render doesn't.
    for (why, fx) in [("a background blur of 60", blur(60.0)), ("glass", glass)] {
        for (from, by) in [((180.0, 100.0), (15.0, 10.0)), ((-150.0, 100.0), (20.0, 0.0))] {
            let mut d = Document::new("edge");
            let p = d.first_page().unwrap();
            for k in 0..20 {
                let s = d.create(p, NodeKind::Rectangle, "S", -200.0 + f64::from(k) * 20.0, 0.0, 10.0, 400.0).unwrap();
                solid(d.node_mut(s).unwrap(), if k % 2 == 0 { "FF0000" } else { "0000FF" });
            }
            let e = d.create(p, NodeKind::Rectangle, "E", -60.0, 50.0, 160.0, 200.0).unwrap();
            let n = d.node_mut(e).unwrap();
            n.fills = vec![Paint { opacity: 0.2, ..Paint::solid(Color::WHITE) }];
            n.effects = vec![fx.clone()];
            let m = d.create(p, NodeKind::Rectangle, "M", from.0, from.1, 40.0, 40.0).unwrap();
            solid(d.node_mut(m).unwrap(), "00AA00");
            let before = Arc::new(d);
            let after = edit(&before, |d| d.node_mut(m).unwrap().transform = Affine::translate((from.0 + by.0, from.1 + by.1)));
            let v = View { origin: (0.0, 0.0), scale: 1.0, width: 400, height: 300 };
            let mut r = Retained::default();
            r.render(&before, p, &v, &Options::default()).unwrap();
            let (shown, pix, work) = r.render(&after, p, &v, &Options::default()).unwrap();
            let pix = pix.clone();
            let full = render_view(&after, p, &shown, &Options::default()).unwrap();
            let worst = pix.data().iter().zip(full.data()).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            assert!(worst <= 2, "{why}, a square moved from {from:?}: {work:?} is {worst} levels off a full render");
        }
    }
}

#[test]
fn pans_patch_backdrops_at_the_edges_that_moved_in() {
    fonts();
    let mut blur = Effect::background_blur();
    if let EffectKind::BackgroundBlur(b) = &mut blur.kind {
        b.radius = 10.0;
    }
    let mut glass = Effect::glass();
    if let EffectKind::Glass(g) = &mut glass.kind {
        (g.depth, g.refraction) = (30.0, 1.0);
    }
    // A layer reading its backdrop across all four edges of the view, over stripes: after a pan,
    // what it read past an edge that moved in is past the edge no more.
    for (why, fx) in [("a background blur of 10", blur), ("glass", glass)] {
        let mut d = Document::new("pan");
        let p = d.first_page().unwrap();
        for k in 0..40 {
            let s = d.create(p, NodeKind::Rectangle, "S", -200.0 + f64::from(k) * 20.0, -100.0, 10.0, 600.0).unwrap();
            solid(d.node_mut(s).unwrap(), if k % 2 == 0 { "FF0000" } else { "0000FF" });
        }
        let e = d.create(p, NodeKind::Rectangle, "E", -60.0, -60.0, 500.0, 420.0).unwrap();
        let n = d.node_mut(e).unwrap();
        n.fills = vec![Paint { opacity: 0.2, ..Paint::solid(Color::WHITE) }];
        n.effects = vec![fx];
        let doc = Arc::new(d);
        for (dx, dy) in [(30.0, 0.0), (-30.0, 0.0), (0.0, 30.0), (0.0, -30.0)] {
            let mut r = Retained::default();
            r.render(&doc, p, &View { origin: (0.0, 0.0), scale: 1.0, width: 400, height: 300 }, &Options::default()).unwrap();
            let v = View { origin: (dx, dy), scale: 1.0, width: 400, height: 300 };
            let (shown, pix, work) = r.render(&doc, p, &v, &Options::default()).unwrap();
            let pix = pix.clone();
            let full = render_view(&doc, p, &shown, &Options::default()).unwrap();
            let worst = pix.data().iter().zip(full.data()).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            assert!(partial(work), "{why}, a pan by ({dx}, {dy}): {work:?}");
            assert!(worst <= 2, "{why}, a pan by ({dx}, {dy}): {work:?} is {worst} levels off a full render");
        }
    }
}
