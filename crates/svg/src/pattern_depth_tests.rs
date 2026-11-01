//! A pattern fill's source exports one level below the layer it fills, as vectors and as an
//! image, so one depth cap bounds the whole export (#1082).

use base64::Engine;
use figcraft_color::{Color, Paint, PaintKind, PatternPaint};
use figcraft_doc::{Document, Effect, NodeId, NodeKind};

use crate::{SvgOptions, export_svg};

/// `levels` frames under `parent`, each inside the last (with drop shadows when `shadowed`); the
/// outermost and the innermost.
fn chain(d: &mut Document, parent: NodeId, levels: usize, shadowed: bool) -> (NodeId, NodeId) {
    let mut at = parent;
    let mut first = None;
    for _ in 0..levels {
        at = d.create(at, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
        let n = d.get_mut(at).unwrap();
        n.fills.clear();
        if shadowed {
            n.effects = vec![Effect::of_type("DROP_SHADOW").unwrap()];
        }
        first.get_or_insert(at);
    }
    (first.unwrap(), at)
}

/// The export of an `outer`-deep chain ending in a frame filled with a pattern of another chain,
/// `inner` deep and ending in red.
fn export(outer: usize, inner: usize, shadowed: bool) -> String {
    let mut d = Document::new("depth");
    let page = d.first_page().unwrap();
    let sources = d.add_page("Sources");
    let (source, red) = chain(&mut d, sources, inner, shadowed);
    d.get_mut(red).unwrap().fills = vec![Paint::solid(Color::from_hex("FF0000").unwrap())];
    let (root, filled) = chain(&mut d, page, outer, false);
    d.get_mut(filled).unwrap().fills = vec![Paint { kind: PaintKind::Pattern(PatternPaint::new(source.0)), ..Paint::solid(Color::WHITE) }];
    export_svg(&d, &[root], &SvgOptions::default()).unwrap()
}

/// Whether an image embedded in `svg` has a red pixel.
fn red_image(svg: &str) -> bool {
    svg.split("base64,").skip(1).filter_map(|s| s.split('"').next()).any(|b64| {
        let png = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        let pix = figcraft_render::images::decode(&png).unwrap();
        pix.pixels().iter().any(|p| p.red() > 200 && p.green() < 60 && p.alpha() > 200)
    })
}

#[test]
fn a_pattern_source_exports_from_the_depth_of_the_layer_it_fills() {
    assert!(export(200, 40, false).contains("#FF0000"), "within the cap");
    // It used to start again at depth 0 and export all of it.
    assert!(!export(200, 100, false).contains("#FF0000"), "the levels past the cap are left out");
    assert!(export(1, 100, false).contains("#FF0000"));
}

#[test]
fn a_pattern_cell_drawn_as_an_image_goes_on_from_the_layers_depth() {
    // A source with effects is drawn by the renderer as an image of the cell.
    assert!(red_image(&export(1, 100, true)));
    assert!(!red_image(&export(200, 100, true)), "the renderer counts from the exporter's depth");
}
