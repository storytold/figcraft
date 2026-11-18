use super::outline::stroke;
use super::overlays::accent;
use super::{Align2, Color32, CornerRadius, Document, FigApp, NodeId, NodeKind, Pos2, Rect, Tokens, pos2, regular, semibold, tr, vec2};

/// The layers that get canvas labels and layout grids, in z-order: the page's visible top-level
/// layers and, inside sections whose contents show, their visible children (sections nest). A
/// section comes after its children, so its title pill draws over (and takes clicks from) the
/// labels of frames near its top-left (#1150).
pub(super) fn boards(doc: &Document, page: NodeId) -> Vec<NodeId> {
    fn walk(doc: &Document, id: NodeId, depth: usize, out: &mut Vec<NodeId>) {
        let Some(n) = doc.node(id) else { return };
        for c in &n.children {
            let Some(cn) = doc.node(*c) else { continue };
            if !cn.visible {
                continue;
            }
            if matches!(cn.kind, NodeKind::Section { contents_hidden: false }) && depth < 32 {
                walk(doc, *c, depth + 1, out);
            }
            out.push(*c);
        }
    }
    let mut out = Vec::new();
    walk(doc, page, 0, &mut out);
    out
}

/// A frame's or section's handoff status as its badge says it, with the badge colour ("Changed"
/// once a ready-for-dev layer was edited; `None` without a status).
pub(crate) fn status_label(n: &figcraft_doc::Node) -> Option<(&'static str, Color32)> {
    match n.dev_status {
        figcraft_doc::DevStatus::None => None,
        figcraft_doc::DevStatus::ReadyForDev if n.dev_changed => Some((tr("Changed"), crate::theme::DEV_CHANGED)),
        figcraft_doc::DevStatus::ReadyForDev => Some((tr("Ready for dev"), crate::theme::DEV_READY)),
        figcraft_doc::DevStatus::Completed => Some((tr("Completed"), crate::theme::DEV_COMPLETED)),
    }
}

/// A frame's or section's handoff status as a small pill left-centred on `at`; the pill's area.
pub(crate) fn status_badge(painter: &egui::Painter, n: &figcraft_doc::Node, at: Pos2) -> Option<Rect> {
    let (label, fill) = status_label(n)?;
    let galley = painter.layout_no_wrap(label.to_string(), semibold(10.0), Color32::WHITE);
    let r = Rect::from_min_size(pos2(at.x, at.y - galley.size().y / 2.0 - 2.0), galley.size() + vec2(10.0, 4.0));
    painter.rect_filled(r, CornerRadius::same(4), fill);
    painter.galley(r.min + vec2(5.0, 2.0), galley, Color32::WHITE);
    Some(r)
}

pub(super) fn rulers(app: &FigApp, painter: &egui::Painter, t: &Tokens, doc: &Document, sel: &[NodeId]) {
    let r = app.canvas.rect;
    let size = 20.0;
    painter.rect_filled(Rect::from_min_size(r.min, vec2(r.width(), size)), CornerRadius::ZERO, t.panel);
    painter.rect_filled(Rect::from_min_size(r.min, vec2(size, r.height())), CornerRadius::ZERO, t.panel);
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
    let step = figcraft_engine::guides::ruler_step(zoom);
    let p0 = app.to_page(r.min);
    let p1 = app.to_page(r.max);
    // Highlight the selection's span.
    if let Some(b) = doc.bounds_of(sel) {
        let (a, bb) = (app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1));
        painter.rect_filled(
            Rect::from_min_max(pos2(a.x, r.min.y), pos2(bb.x, r.min.y + size)),
            CornerRadius::ZERO,
            accent(painter).gamma_multiply(0.15),
        );
        painter.rect_filled(
            Rect::from_min_max(pos2(r.min.x, a.y), pos2(r.min.x + size, bb.y)),
            CornerRadius::ZERO,
            accent(painter).gamma_multiply(0.15),
        );
    }
    let mut x = (p0.x / step).floor() * step;
    while x <= p1.x {
        let s = app.to_screen(x, 0.0).x;
        painter.line_segment([pos2(s, r.min.y + size - 6.0), pos2(s, r.min.y + size)], stroke(t.text_tertiary, 1.0));
        painter.text(pos2(s + 2.0, r.min.y + 3.0), Align2::LEFT_TOP, fmt(x), regular(9.0), t.text_secondary);
        x += step;
    }
    let mut y = (p0.y / step).floor() * step;
    while y <= p1.y {
        let s = app.to_screen(0.0, y).y;
        painter.line_segment([pos2(r.min.x + size - 6.0, s), pos2(r.min.x + size, s)], stroke(t.text_tertiary, 1.0));
        painter.text(pos2(r.min.x + 3.0, s + 2.0), Align2::LEFT_TOP, fmt(y), regular(9.0), t.text_secondary);
        y += step;
    }
    painter.line_segment([pos2(r.min.x, r.min.y + size), pos2(r.max.x, r.min.y + size)], stroke(t.border, 1.0));
    painter.line_segment([pos2(r.min.x + size, r.min.y), pos2(r.min.x + size, r.max.y)], stroke(t.border, 1.0));
}

pub fn fmt(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-9 { format!("{}", r.round() as i64) } else { format!("{r}") }
}
