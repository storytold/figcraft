//! What the retained renderer works out about a page (grainy layers, pattern sources, the largest
//! backdrop blur) comes from that page and the pattern sources it draws, not from the whole file.

use std::sync::Arc;

use figcraft_color::{Color, Paint, PaintKind, PatternPaint};
use figcraft_doc::{Blur, Document, Effect, EffectKind, NodeId, NodeKind};
use figcraft_geom::Affine;

use crate::retained_tests::assert_same;
use crate::{Options, Pixmap, Retained, View, Work, render_view};

fn solid(d: &mut Document, id: NodeId, hex: &str) {
    d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex(hex).unwrap())];
}

fn pattern(d: &mut Document, id: NodeId, source: NodeId) {
    let pt = PatternPaint::new(source.0);
    d.get_mut(id).unwrap().fills = vec![Paint { kind: PaintKind::Pattern(pt), ..Paint::solid(Color::BLACK) }];
}

fn edit(doc: &Arc<Document>, f: impl FnOnce(&mut Document)) -> Arc<Document> {
    let mut d = (**doc).clone();
    f(&mut d);
    Arc::new(d)
}

/// Render through `r`; the image, the view drawn and what it took.
fn render(r: &mut Retained, doc: &Arc<Document>, page: NodeId, v: &View) -> (Pixmap, View, Work) {
    let (shown, pix, work) = r.render(doc, page, v, &Options::default()).unwrap();
    (pix.clone(), shown, work)
}

/// Render through `r` and check the image against a full render of the same view: the same
/// pixels for a full render, the same up to anti-aliasing for a patched one.
fn check(r: &mut Retained, doc: &Arc<Document>, page: NodeId, v: &View) -> Work {
    let (pix, shown, work) = render(r, doc, page, v);
    let full = render_view(doc, page, &shown, &Options::default()).unwrap();
    if work == Work::Full {
        assert!(pix.data() == full.data(), "a full retained render differs from render_view");
    } else {
        assert_same(&pix, &full, &format!("{work:?}"));
    }
    work
}

/// Shapes on the first page, and a background blur reaching far past any patch's margin on the
/// second: (document, first page, a shape on it).
fn blur_elsewhere() -> (Arc<Document>, NodeId, NodeId) {
    let mut d = Document::new("blur elsewhere");
    let p = d.first_page().unwrap();
    let mut shape = None;
    for i in 0..12u32 {
        let r = d.create(p, NodeKind::Rectangle, "R", f64::from(i % 4) * 160.0 + 20.0, f64::from(i / 4) * 150.0 + 20.0, 120.0, 100.0).unwrap();
        solid(&mut d, r, ["E03030", "2040E0", "20C060", "F0C830"][i as usize % 4]);
        shape.get_or_insert(r);
    }
    let other = d.add_page("Other");
    let frosted = d.create(other, NodeKind::Rectangle, "Frosted", 0.0, 0.0, 400.0, 300.0).unwrap();
    d.get_mut(frosted).unwrap().effects = vec![Effect { kind: EffectKind::BackgroundBlur(Blur::uniform(1000.0)), visible: true }];
    (Arc::new(d), p, shape.unwrap())
}

#[test]
fn a_blur_on_another_page_leaves_pans_and_edits_partial() {
    let (doc, p, shape) = blur_elsewhere();
    let v = |x: f64| View { origin: (x, 0.0), scale: 1.0, width: 700, height: 500 };
    let mut r = Retained::default();
    assert_eq!(check(&mut r, &doc, p, &v(0.0)), Work::Full);
    assert!(matches!(check(&mut r, &doc, p, &v(30.0)), Work::Partial { .. }), "a pan");
    let moved = edit(&doc, |d| d.get_mut(shape).unwrap().transform = Affine::translate((60.0, 40.0)));
    assert!(matches!(check(&mut r, &moved, p, &v(30.0)), Work::Partial { .. }), "an edit");
    // A new zoom renders in bands as wide apart as the page's own blurs need (none here).
    assert_eq!(check(&mut r, &moved, p, &View { scale: 1.5, ..v(30.0) }), Work::Full);
}

#[test]
fn a_blur_on_the_page_still_renders_full_when_it_reaches_past_a_patch() {
    let (doc, p, shape) = blur_elsewhere();
    // The same blur on the page shown.
    let doc = edit(&doc, |d| {
        let frosted = d.create(p, NodeKind::Rectangle, "Frosted", 300.0, 200.0, 200.0, 150.0).unwrap();
        d.get_mut(frosted).unwrap().effects = vec![Effect { kind: EffectKind::BackgroundBlur(Blur::uniform(1000.0)), visible: true }];
    });
    let v = View { origin: (0.0, 0.0), scale: 1.0, width: 700, height: 500 };
    let mut r = Retained::default();
    check(&mut r, &doc, p, &v);
    let moved = edit(&doc, |d| d.get_mut(shape).unwrap().transform = Affine::translate((60.0, 40.0)));
    assert_eq!(check(&mut r, &moved, p, &v), Work::Full);
}

/// A rectangle on the first page filled with a pattern whose source is a frame on the second
/// page; inside the source, a child and a layer filled with a second pattern, whose source is on
/// a third page. (document, first page, filled rectangle, source, child, nested source)
fn patterns() -> (Arc<Document>, NodeId, [NodeId; 4]) {
    let mut d = Document::new("patterns");
    let p = d.first_page().unwrap();
    let filled = d.create(p, NodeKind::Rectangle, "Filled", 20.0, 20.0, 260.0, 200.0).unwrap();
    let plain = d.create(p, NodeKind::Rectangle, "Plain", 300.0, 40.0, 60.0, 60.0).unwrap();
    solid(&mut d, plain, "202020");
    let second = d.add_page("Sources");
    let source = d.create(second, NodeKind::Frame, "Source", 0.0, 0.0, 40.0, 40.0).unwrap();
    solid(&mut d, source, "E03030");
    let child = d.create(source, NodeKind::Rectangle, "Child", 4.0, 4.0, 14.0, 14.0).unwrap();
    solid(&mut d, child, "2040E0");
    let inner = d.create(source, NodeKind::Rectangle, "Inner", 20.0, 20.0, 18.0, 18.0).unwrap();
    let third = d.add_page("More sources");
    let nested = d.create(third, NodeKind::Rectangle, "Nested", 0.0, 0.0, 6.0, 6.0).unwrap();
    solid(&mut d, nested, "20C060");
    pattern(&mut d, inner, nested);
    pattern(&mut d, filled, source);
    (Arc::new(d), p, [filled, source, child, nested])
}

#[test]
fn edits_to_pattern_sources_on_other_pages_show_on_the_page() {
    let (doc, p, [_, source, child, nested]) = patterns();
    let v = View { origin: (0.0, 0.0), scale: 1.0, width: 400, height: 260 };
    let mut r = Retained::default();
    check(&mut r, &doc, p, &v);
    let mut doc = doc;
    for (what, id, hex) in
        [("the source", source, "F0C830"), ("a layer in the source", child, "FFFFFF"), ("a source the source fills with", nested, "000000")]
    {
        doc = edit(&doc, |d| solid(d, id, hex));
        assert_ne!(check(&mut r, &doc, p, &v), Work::Reused, "{what}");
    }
}

#[test]
fn a_grainy_layer_patches_as_it_renders_after_a_zoom() {
    let mut d = Document::new("grain");
    let p = d.first_page().unwrap();
    let grainy = d.create(p, NodeKind::Rectangle, "Grain", 60.0, 50.0, 120.0, 90.0).unwrap();
    solid(&mut d, grainy, "E03030");
    let noise = figcraft_doc::Noise { size: 0.5, density: 0.4, color: Color::BLACK, ..Default::default() };
    d.get_mut(grainy).unwrap().effects = vec![Effect { kind: EffectKind::Noise(noise), visible: true }];
    let other = d.add_page("Other");
    let far = d.create(other, NodeKind::Rectangle, "Far", 0.0, 0.0, 10.0, 10.0).unwrap();
    solid(&mut d, far, "2040E0");
    let doc = Arc::new(d);
    let v = |x: f64, scale: f64| View { origin: (x, 0.0), scale, width: 400, height: 300 };
    let mut r = Retained::default();
    check(&mut r, &doc, p, &v(0.0, 1.0));
    assert_eq!(check(&mut r, &doc, p, &v(0.0, 1.7)), Work::Full);
    // Pans across the grainy layer at the new zoom: it is redrawn whole each time.
    for i in 1..=4 {
        let (pix, shown, work) = render(&mut r, &doc, p, &v(f64::from(i) * 13.0, 1.7));
        let full = render_view(&doc, p, &shown, &Options::default()).unwrap();
        let worst = pix.data().iter().zip(full.data()).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert!(matches!(work, Work::Partial { .. }), "{work:?}");
        assert!(worst <= 2, "pan {i}: {worst} levels off a full render");
    }
}

#[test]
fn a_page_shown_again_after_another_matches() {
    let (doc, p, [filled, ..]) = patterns();
    let other = doc.pages[1];
    let v = |x: f64| View { origin: (x, 0.0), scale: 1.0, width: 400, height: 260 };
    let mut r = Retained::default();
    check(&mut r, &doc, p, &v(0.0));
    assert_eq!(check(&mut r, &doc, other, &v(0.0)), Work::Full);
    assert_eq!(check(&mut r, &doc, p, &v(0.0)), Work::Full);
    assert!(matches!(check(&mut r, &doc, p, &v(24.0)), Work::Partial { .. }));
    let moved = edit(&doc, |d| d.get_mut(filled).unwrap().transform = Affine::translate((40.0, 30.0)));
    assert!(matches!(check(&mut r, &moved, p, &v(24.0)), Work::Partial { .. }));
}

#[test]
fn a_layer_listed_as_its_own_child_ends_the_page_walk() {
    let mut d = Document::new("loop");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 50.0, 50.0).unwrap();
    solid(&mut d, f, "E03030");
    d.get_mut(f).unwrap().children.push(f);
    let doc = Arc::new(d);
    let mut r = Retained::default();
    let v = View { origin: (0.0, 0.0), scale: 1.0, width: 100, height: 100 };
    assert!(r.render(&doc, p, &v, &Options::default()).is_some());
    assert!(r.render(&doc, p, &View { origin: (5.0, 0.0), ..v }, &Options::default()).is_some());
}
