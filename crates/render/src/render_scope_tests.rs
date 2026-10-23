//! Only effects participating in the active page affect its band/patch margins. Pattern sources
//! remain dependencies even when they live under hidden ancestors or on another page.

use std::sync::Arc;

use figcraft_color::{Color, Paint, PaintKind, PatternPaint};
use figcraft_doc::{Blur, Document, Effect, EffectKind, NodeId, NodeKind, Noise};
use figcraft_geom::Affine;

use crate::{Options, Retained, View, Work, max_backdrop_blur, render_view};

fn rectangle(d: &mut Document, parent: NodeId, x: f64, y: f64, width: f64, height: f64) -> NodeId {
    let id = d.create(parent, NodeKind::Rectangle, "Rectangle", x, y, width, height).unwrap();
    d.node_mut(id).unwrap().fills = vec![Paint::solid(Color::new(0.9, 0.2, 0.1, 1.0))];
    id
}

fn blur(d: &mut Document, id: NodeId, radius: f64) {
    d.node_mut(id).unwrap().effects = vec![Effect { kind: EffectKind::BackgroundBlur(Blur::uniform(radius)), visible: true }];
}

fn pattern(d: &mut Document, id: NodeId, source: NodeId) {
    d.node_mut(id).unwrap().fills = vec![Paint { kind: PaintKind::Pattern(PatternPaint::new(source.0)), ..Paint::solid(Color::WHITE) }];
}

fn view() -> View {
    View { origin: (0.0, 0.0), scale: 1.0, width: 400, height: 300 }
}

fn check(retained: &mut Retained, doc: &Arc<Document>, page: NodeId, v: &View, opts: &Options) -> Work {
    // Other render tests use text. Finish the process-wide font scan before keeping a frame, so
    // its cache-generation change cannot race these Work assertions.
    figcraft_text::scan_system_fonts();
    let (shown, pix, work) = retained.render(doc, page, v, opts).unwrap();
    let full = render_view(doc, page, &shown, opts).unwrap();
    crate::retained_tests::assert_same(pix, &full, &format!("scoped effects: {work:?}"));
    work
}

fn edit(doc: &Arc<Document>, f: impl FnOnce(&mut Document)) -> Arc<Document> {
    let mut d = (**doc).clone();
    f(&mut d);
    Arc::new(d)
}

#[test]
fn backdrop_reach_follows_page_visibility_skip_solo_and_outline() {
    let mut d = Document::new("scoped effects");
    let page = d.first_page().unwrap();
    let active = rectangle(&mut d, page, 20.0, 20.0, 20.0, 20.0);
    blur(&mut d, active, 4.0);
    let hidden = d.create(page, NodeKind::Group, "Hidden", 0.0, 0.0, 80.0, 80.0).unwrap();
    d.node_mut(hidden).unwrap().visible = false;
    let child = rectangle(&mut d, hidden, 0.0, 0.0, 80.0, 80.0);
    blur(&mut d, child, 2000.0);
    let transparent = d.create(page, NodeKind::Group, "Transparent", 0.0, 0.0, 80.0, 80.0).unwrap();
    d.node_mut(transparent).unwrap().opacity = 0.0;
    let c = rectangle(&mut d, transparent, 0.0, 0.0, 80.0, 80.0);
    blur(&mut d, c, 3000.0);
    let section = d.create(page, NodeKind::Section { contents_hidden: true }, "Collapsed", 0.0, 0.0, 80.0, 80.0).unwrap();
    let c = rectangle(&mut d, section, 0.0, 0.0, 80.0, 80.0);
    blur(&mut d, c, 4000.0);
    let other_page = d.add_page("Other page");
    let other = rectangle(&mut d, other_page, 0.0, 0.0, 80.0, 80.0);
    blur(&mut d, other, 5000.0);
    let opts = Options::default();
    assert_eq!(max_backdrop_blur(&d, page, &opts), 4.0);
    assert_eq!(max_backdrop_blur(&d, other_page, &opts), 5000.0);
    assert_eq!(max_backdrop_blur(&d, page, &Options { skip: vec![active], ..opts.clone() }), 0.0);
    assert_eq!(max_backdrop_blur(&d, page, &Options { outlined: vec![active], ..opts.clone() }), 0.0);
    assert_eq!(max_backdrop_blur(&d, page, &Options { outline_mode: true, ..opts.clone() }), 0.0);
    // Solo draws a descendant directly, independently of its hidden parent.
    assert_eq!(max_backdrop_blur(&d, page, &Options { solo: Some(child), ..opts.clone() }), 2000.0);
    assert_eq!(max_backdrop_blur(&d, page, &Options { solo: Some(other), ..opts.clone() }), 4.0);
    d.node_mut(hidden).unwrap().visible = true;
    assert_eq!(max_backdrop_blur(&d, page, &opts), 2000.0);
    assert_eq!(max_backdrop_blur(&d, page, &Options { skip: vec![hidden], ..opts }), 4.0);
}

#[test]
fn hidden_and_other_page_effects_do_not_force_visible_edits_or_pans_full() {
    let mut d = Document::new("inactive effects");
    let page = d.first_page().unwrap();
    let active = rectangle(&mut d, page, 20.0, 20.0, 30.0, 20.0);
    let hidden = d.create(page, NodeKind::Group, "Hidden", 0.0, 0.0, 800.0, 600.0).unwrap();
    d.node_mut(hidden).unwrap().visible = false;
    let noisy = rectangle(&mut d, hidden, 0.0, 0.0, 800.0, 600.0);
    d.node_mut(noisy).unwrap().effects = vec![Effect { kind: EffectKind::Noise(Noise::default()), visible: true }];
    // Its distance also proves we retain conservative page-wide reach once it becomes visible.
    let huge = rectangle(&mut d, hidden, 30_000.0, 30_000.0, 20.0, 20.0);
    blur(&mut d, huge, 2000.0);
    let other_page = d.add_page("Other page");
    let other = rectangle(&mut d, other_page, 0.0, 0.0, 400.0, 300.0);
    blur(&mut d, other, 5000.0);
    let doc = Arc::new(d);
    let mut retained = Retained::default();
    let opts = Options::default();
    assert_eq!(check(&mut retained, &doc, page, &view(), &opts), Work::Full);
    assert_eq!(check(&mut retained, &doc, page, &view(), &opts), Work::Reused);
    let moved = edit(&doc, |d| d.node_mut(active).unwrap().transform = Affine::translate((25.0, 22.0)));
    assert!(matches!(check(&mut retained, &moved, page, &view(), &opts), Work::Partial { .. }));
    let pan = View { origin: (5.0, 3.0), ..view() };
    assert!(matches!(check(&mut retained, &moved, page, &pan, &opts), Work::Partial { .. }));
    // Showing a hidden large blur restores the conservative full-render path; hiding it again
    // clears the old frame safely before subsequent ordinary edits can be partial again.
    let shown = edit(&moved, |d| {
        d.node_mut(hidden).unwrap().visible = true;
        d.node_mut(noisy).unwrap().visible = false;
    });
    assert_eq!(check(&mut retained, &shown, page, &pan, &opts), Work::Full);
    let hidden_again = edit(&shown, |d| d.node_mut(hidden).unwrap().visible = false);
    assert_eq!(check(&mut retained, &hidden_again, page, &pan, &opts), Work::Full);
    let moved = edit(&hidden_again, |d| d.node_mut(active).unwrap().transform = Affine::translate((28.0, 24.0)));
    assert!(matches!(check(&mut retained, &moved, page, &pan, &opts), Work::Partial { .. }));
}

#[test]
fn visible_pattern_keeps_hidden_ancestor_and_other_page_source_dependencies() {
    for nested in [false, true] {
        let mut d = Document::new("pattern dependencies");
        let page = d.first_page().unwrap();
        let target = rectangle(&mut d, page, 20.0, 20.0, 100.0, 80.0);
        let other = d.add_page("Sources");
        let hidden = d.create(other, NodeKind::Group, "Hidden sources", 0.0, 0.0, 100.0, 100.0).unwrap();
        d.node_mut(hidden).unwrap().visible = false;
        let source = rectangle(&mut d, hidden, 0.0, 0.0, 12.0, 12.0);
        if nested {
            let middle = rectangle(&mut d, hidden, 20.0, 0.0, 24.0, 24.0);
            pattern(&mut d, middle, source);
            pattern(&mut d, target, middle);
        } else {
            pattern(&mut d, target, source);
        }
        let doc = Arc::new(d);
        let mut retained = Retained::default();
        check(&mut retained, &doc, page, &view(), &Options::default());
        let changed = edit(&doc, |d| d.node_mut(source).unwrap().fills = vec![Paint::solid(Color::new(0.0, 0.2, 0.9, 1.0))]);
        assert_eq!(check(&mut retained, &changed, page, &view(), &Options::default()), Work::Full, "pattern source nested={nested}");
        // The source visibly participates despite its hidden ancestor on another page.
        let before = render_view(&doc, page, &view(), &Options::default()).unwrap();
        let after = render_view(&changed, page, &view(), &Options::default()).unwrap();
        assert_ne!(before.data(), after.data());
    }
}

#[test]
fn unreferenced_hidden_pattern_paints_do_not_invalidate_the_active_page() {
    let mut d = Document::new("unused pattern");
    let page = d.first_page().unwrap();
    rectangle(&mut d, page, 20.0, 20.0, 30.0, 20.0);
    let other = d.add_page("Other page");
    let source = rectangle(&mut d, other, 0.0, 0.0, 12.0, 12.0);
    let hidden = rectangle(&mut d, page, 20.0, 20.0, 100.0, 80.0);
    pattern(&mut d, hidden, source);
    d.node_mut(hidden).unwrap().visible = false;
    let offpage = rectangle(&mut d, other, 0.0, 0.0, 100.0, 100.0);
    pattern(&mut d, offpage, source);
    let doc = Arc::new(d);
    let mut retained = Retained::default();
    check(&mut retained, &doc, page, &view(), &Options::default());
    let changed = edit(&doc, |d| d.node_mut(source).unwrap().fills = vec![Paint::solid(Color::WHITE)]);
    assert_eq!(check(&mut retained, &changed, page, &view(), &Options::default()), Work::Reused);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn inactive_large_blurs_do_not_disable_parallel_bands() {
    let mut d = Document::new("band scope");
    let page = d.first_page().unwrap();
    rectangle(&mut d, page, 20.0, 90.0, 80.0, 100.0);
    let other = d.add_page("Other page");
    let source = rectangle(&mut d, other, 0.0, 0.0, 100.0, 100.0);
    blur(&mut d, source, 5000.0);
    let v = View { width: 800, height: 600, ..view() };
    let opts = Options::default();
    let bands = crate::render_bands(&d, page, &v, &opts, 4).expect("inactive blur must not force a serial render");
    let single = crate::render_view_single(&d, page, &v, &opts).unwrap();
    crate::retained_tests::assert_same(&bands, &single, "scoped parallel bands");
}
