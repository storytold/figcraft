//! Other people's text carets in a live session: while someone types in a text layer of the
//! shared document, their caret and selected characters show on it in their colour, at the glyph
//! positions of your copy of the text (the text layout's own caret and selection geometry), with
//! their name above the caret; when you type in the same layer the tag says so. Remote carets
//! don't blink: they draw only when the canvas draws anyway (a presence update wakes it), so an
//! idle session asks for no frames.

use egui::{Align2, Shape, Stroke, vec2};
use figcraft_engine::live::editing_hint;

use crate::FigApp;
use crate::canvas::TextOnScreen;
use crate::live::{color, name_tag};

/// How strongly someone else's selection tints the characters.
const SELECTION_ALPHA: f32 = 0.25;
/// Width of someone else's caret (points).
const CARET_WIDTH: f32 = 2.0;

/// Draw the carets and selections of everyone typing in a text layer on your page.
pub fn draw(app: &FigApp, painter: &egui::Painter) {
    let live = app.session.live();
    if live.presence.is_empty() || !app.session.live_shared() {
        return;
    }
    let Ok(ds) = app.session.ds() else { return };
    let mine = ds.text_edit.as_ref().map(|t| t.id);
    for (id, p) in &live.presence {
        let (Some(peer), Some(e)) = (live.peers.get(id), p.editing_text.as_deref()) else { continue };
        if ds.doc.page_of(e.node) != Some(ds.page) {
            continue;
        }
        let Some(text) = TextOnScreen::of(&ds.doc, e.node) else { continue };
        let fill = color(&peer.color);
        // Positions count characters of their copy (typing syncs when they finish): kept within
        // the text you have.
        let at = |i: usize| i.min(text.chars);
        if let Some((a, b)) = e.selection.map(|(a, b)| (at(a), at(b))).filter(|(a, b)| a < b) {
            for q in text.selection(app, a, b) {
                painter.add(Shape::convex_polygon(q.to_vec(), fill.gamma_multiply(SELECTION_ALPHA), Stroke::NONE));
            }
        }
        let [top, bottom] = text.caret(app, at(e.caret), ds.view.zoom);
        painter.line_segment([top, bottom], Stroke::new(CARET_WIDTH, fill));
        let label = if mine == Some(e.node) { editing_hint(&[peer.name.as_str()]).unwrap_or_default() } else { peer.name.clone() };
        name_tag(painter, top.min(bottom) + vec2(-CARET_WIDTH / 2.0, -2.0), Align2::LEFT_BOTTOM, &label, fill);
    }
}
