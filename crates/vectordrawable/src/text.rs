//! Text layers as glyph outlines, one path per styled run.

use figcraft_color::Paint;
use figcraft_doc::{Node, NodeKind};
use figcraft_geom::{Affine, BezPath};

/// Text layer `n`'s glyph and decoration outlines grouped by style, in layer space, each with the
/// fills it paints in (its own, else the layer's). `None` for other layers and text that doesn't
/// lay out.
pub(crate) fn runs(n: &Node) -> Option<Vec<(BezPath, Vec<Paint>)>> {
    let NodeKind::Text(_) = &n.kind else { return None };
    let (layout, dy) = figcraft_text::layout_node(n)?;
    let mut by_style: std::collections::BTreeMap<usize, BezPath> = std::collections::BTreeMap::new();
    for g in &layout.glyphs {
        by_style.entry(g.style).or_default().extend(g.path().elements().iter().copied());
    }
    for d in &layout.decorations {
        by_style.entry(d.style).or_default().extend(d.path().elements().iter().copied());
    }
    Some(
        by_style
            .into_iter()
            .map(|(k, mut path)| {
                path.apply_affine(Affine::translate((0.0, dy)));
                let fills = layout.styles.get(k).and_then(|s| s.fills.clone()).unwrap_or_else(|| n.fills.clone());
                (path, fills)
            })
            .collect(),
    )
}
