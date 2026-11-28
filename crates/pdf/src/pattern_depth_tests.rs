//! A pattern fill's source exports one level below the layer it fills, so one depth cap bounds
//! the whole export (#1082).

use figcraft_color::{Color, Paint, PaintKind, PatternPaint};
use figcraft_doc::{Document, NodeId, NodeKind};

use crate::tests::inflated;
use crate::{PdfOptions, export_pdf};

/// `levels` frames under `parent`, each inside the last; the outermost and the innermost.
fn chain(d: &mut Document, parent: NodeId, levels: usize) -> (NodeId, NodeId) {
    let mut at = parent;
    let mut first = None;
    for _ in 0..levels {
        at = d.create(at, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
        d.get_mut(at).unwrap().fills.clear();
        first.get_or_insert(at);
    }
    (first.unwrap(), at)
}

/// The export of an `outer`-deep chain ending in a frame filled with a pattern of another chain,
/// `inner` deep and ending in red.
fn export(outer: usize, inner: usize) -> String {
    let mut d = Document::new("depth");
    let page = d.first_page().unwrap();
    let sources = d.add_page("Sources");
    let (source, red) = chain(&mut d, sources, inner);
    d.get_mut(red).unwrap().fills = vec![Paint::solid(Color::from_hex("FF0000").unwrap())];
    let (root, filled) = chain(&mut d, page, outer);
    d.get_mut(filled).unwrap().fills = vec![Paint { kind: PaintKind::Pattern(PatternPaint::new(source.0)), ..Paint::solid(Color::WHITE) }];
    inflated(&export_pdf(&d, &[root], &PdfOptions::default()).unwrap())
}

#[test]
fn a_pattern_source_exports_from_the_depth_of_the_layer_it_fills() {
    assert!(export(200, 40).contains("1 0 0 rg"), "within the cap");
    // It used to start again at depth 0 and export all of it.
    assert!(!export(200, 100).contains("1 0 0 rg"), "the levels past the cap are left out");
    assert!(export(1, 100).contains("1 0 0 rg"));
}
