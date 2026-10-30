//! One depth cap for the whole render (#1082): a pattern fill's source is drawn one level below
//! the layer it fills, not as a fresh render, so patterns in patterns can't stack three times the
//! cap; and the deepest render the cap allows fits a render thread's stack ([`RENDER_STACK`]).

use figcraft_color::{Color, Paint, PaintKind, PatternPaint};
use figcraft_doc::{Document, Effect, NodeId, NodeKind};
use tiny_skia::Pixmap;

use crate::{MAX_EXPORT_DEPTH, Options, RENDER_STACK, View, render_view, render_view_single};

/// `levels` 100 × 100 frames under `parent`, each inside the last (with drop shadows when
/// `heavy`, which keeps every level drawn); the outermost and the innermost.
fn chain(d: &mut Document, parent: NodeId, levels: usize, heavy: bool) -> (NodeId, NodeId) {
    let mut at = parent;
    let mut first = None;
    for _ in 0..levels {
        at = d.create(at, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
        let n = d.get_mut(at).unwrap();
        n.fills.clear();
        if heavy {
            n.effects = vec![Effect::of_type("DROP_SHADOW").unwrap()];
        }
        first.get_or_insert(at);
    }
    (first.unwrap(), at)
}

fn pattern_of(source: NodeId) -> Paint {
    Paint { kind: PaintKind::Pattern(PatternPaint::new(source.0)), ..Paint::solid(Color::WHITE) }
}

/// A page whose `outer`-deep chain ends in a frame filled with a pattern of another chain,
/// `inner` deep and ending in red (kept on a second page).
fn pattern_at_depth(outer: usize, inner: usize) -> (Document, NodeId) {
    let mut d = Document::new("depth");
    let page = d.first_page().unwrap();
    let sources = d.add_page("Sources");
    let (source, red) = chain(&mut d, sources, inner, false);
    d.get_mut(red).unwrap().fills = vec![Paint::solid(Color::from_hex("FF0000").unwrap())];
    let (_, filled) = chain(&mut d, page, outer, false);
    d.get_mut(filled).unwrap().fills = vec![pattern_of(source)];
    (d, page)
}

fn red_at_centre(pix: &Pixmap) -> bool {
    let p = pix.pixel(50, 50).unwrap();
    p.red() > 200 && p.green() < 60
}

fn render(d: &Document, page: NodeId) -> Pixmap {
    render_view_single(d, page, &View { origin: (0.0, 0.0), scale: 1.0, width: 100, height: 100 }, &Options::default()).unwrap()
}

#[test]
fn a_pattern_source_goes_on_from_the_depth_of_the_layer_it_fills() {
    // 200 levels, then 40 more in the pattern's source: within the cap, so the red shows.
    let (d, page) = pattern_at_depth(200, 40);
    assert!(red_at_centre(&render(&d, page)));
    // 200, then 100 more: past the cap. The source used to start again at depth 0 and draw.
    let (d, page) = pattern_at_depth(200, 100);
    const { assert!(200 + 100 > MAX_EXPORT_DEPTH) };
    assert!(!red_at_centre(&render(&d, page)), "the levels past the cap are not drawn");
    // A pattern at the top of the page still draws its whole source.
    let (d, page) = pattern_at_depth(1, 100);
    assert!(red_at_centre(&render(&d, page)));
}

#[test]
fn patterns_in_patterns_as_deep_as_the_cap_render_on_a_render_threads_stack() {
    let drawn = std::thread::Builder::new()
        .stack_size(RENDER_STACK)
        .spawn(|| {
            // Three chains of 250 heavy levels, each ending in a pattern of the next: what used to
            // be three renders 250 deep on top of each other is one, cut at the cap.
            let mut d = Document::new("patterns");
            let page = d.first_page().unwrap();
            let sources = d.add_page("Sources");
            let (c, c_end) = chain(&mut d, sources, 250, true);
            d.get_mut(c_end).unwrap().fills = vec![Paint::solid(Color::from_hex("FF0000").unwrap())];
            let (b, b_end) = chain(&mut d, sources, 250, true);
            d.get_mut(b_end).unwrap().fills = vec![pattern_of(c)];
            let (_, a_end) = chain(&mut d, page, 250, true);
            d.get_mut(a_end).unwrap().fills = vec![pattern_of(b)];
            let pix = render(&d, page);
            // A view large enough to render in bands, on the band pool.
            let big = render_view(&d, page, &View { origin: (0.0, 0.0), scale: 1.0, width: 600, height: 600 }, &Options::default());
            (red_at_centre(&pix), big.is_some())
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(drawn, (false, true), "rendered, and the red 750 levels down is past the cap");
}
