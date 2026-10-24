//! Text measurement for the layout engine.

use figcraft_doc::{Document, NodeId, NodeKind, TextAutoResize};

pub struct TextMeasure;

impl figcraft_layout::Measure for TextMeasure {
    fn text_height(&self, doc: &Document, id: NodeId, width: f64) -> Option<f64> {
        let _fonts = figcraft_text::enter_document_fonts(doc);
        let n = doc.node(id)?;
        let NodeKind::Text(t) = &n.kind else { return None };
        let mut t = t.clone();
        if t.auto_resize == TextAutoResize::WidthAndHeight {
            t.auto_resize = TextAutoResize::Height;
        }
        Some(figcraft_text::layout(&t, Some(width), None).height.max(1.0))
    }
    fn text_size(&self, doc: &Document, id: NodeId) -> Option<(f64, f64)> {
        let _fonts = figcraft_text::enter_document_fonts(doc);
        let n = doc.node(id)?;
        let NodeKind::Text(t) = &n.kind else { return None };
        if t.auto_resize == TextAutoResize::WidthAndHeight {
            return figcraft_text::auto_size(n);
        }
        // Natural (unwrapped) size, whatever the resize mode.
        let mut t = t.clone();
        t.auto_resize = TextAutoResize::WidthAndHeight;
        let l = figcraft_text::layout_cached(&t, None, None);
        Some((l.width.ceil().max(1.0), l.height.max(1.0)))
    }
    fn text_baseline(&self, doc: &Document, id: NodeId) -> Option<f64> {
        let _fonts = figcraft_text::enter_document_fonts(doc);
        let n = doc.node(id)?;
        let (l, dy) = figcraft_text::layout_node(n)?;
        l.lines.first().map(|line| line.baseline + dy).filter(|b| b.is_finite())
    }
}
