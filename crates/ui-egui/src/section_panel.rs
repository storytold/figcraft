//! Design panel extras for frames and sections: section colours, hiding a section's contents,
//! handoff status (with resolving a change to a ready-for-dev layer) and resize to fit.

use egui::{CornerRadius, Sense, Stroke, Ui, vec2};
use figcraft_doc::{DevStatus, Document, NodeId, NodeKind};
use serde_json::json;

use crate::FigApp;
use crate::i18n::{tr, tr_noop, trf};
use crate::icons::Icon;
use crate::right::{common, menu_item, menu_popup, row};
use crate::widgets;

/// Section fill presets: light tints that keep dark text readable.
const SECTION_COLORS: [(&str, &str); 8] = [
    (tr_noop("White"), "#FFFFFF"),
    (tr_noop("Gray"), "#EDEDED"),
    (tr_noop("Red"), "#FFE0DE"),
    (tr_noop("Orange"), "#FFE8D1"),
    (tr_noop("Yellow"), "#FFF4C2"),
    (tr_noop("Green"), "#DDF5E3"),
    (tr_noop("Blue"), "#DCEBFF"),
    (tr_noop("Violet"), "#EBE3FF"),
];

fn is_section(doc: &Document, id: NodeId) -> bool {
    doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Section { .. }))
}

pub use figcraft_engine::devinspect::has_status;

/// The handoff status of `ids` as a dropdown `width` wide ("Changed" when edited since marked ready,
/// "Mixed" when they differ).
pub fn status_dropdown(app: &mut FigApp, ui: &mut Ui, doc: &Document, ids: &[NodeId], width: f32) {
    let label = match common(doc, ids, |n| (n.dev_status, n.dev_changed)) {
        Some((DevStatus::ReadyForDev, true)) => tr("Changed"),
        Some((d, _)) => tr(d.label()),
        None => tr("Mixed"),
    };
    let resp = widgets::dropdown(ui, label, width, Some(Icon::Code));
    menu_popup(&resp).show(|ui| {
        for d in DevStatus::ALL {
            if menu_item(ui, tr(d.label()), "") {
                app.set_props(ids, json!({"devStatus": d.id()}));
                ui.close();
            }
        }
    });
}

pub fn section(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let sections = !sel.is_empty() && sel.iter().all(|i| is_section(doc, *i));
    if sel.is_empty() || !sel.iter().all(|i| has_status(doc, *i)) {
        return;
    }
    let t = crate::theme::Tokens::get(ui.ctx());
    let ids = sel.to_vec();
    let col = widgets::col_width(ui.available_width());
    if sections {
        widgets::sublabels(ui, tr("Section color"), None);
        row(ui, 32.0, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let current = common(doc, sel, |n| n.fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()));
            for (name, hex) in SECTION_COLORS {
                let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                let c = figcraft_color::Color::from_hex(hex).unwrap_or(figcraft_color::Color::WHITE);
                ui.painter().rect_filled(r, CornerRadius::same(9), crate::theme::doc_c32(ui.ctx(), c));
                let on = current.as_ref().and_then(|c| c.as_ref()).is_some_and(|h| h.eq_ignore_ascii_case(hex.trim_start_matches('#')));
                let stroke = if on { Stroke::new(2.0, t.accent_strong) } else { Stroke::new(1.0, t.border) };
                ui.painter().rect_stroke(r, CornerRadius::same(9), stroke, egui::StrokeKind::Inside);
                let name = tr(name);
                crate::a11y::radio(&resp, name, on);
                if resp.on_hover_text(name).clicked() {
                    app.set_props(&ids, json!({"fills": [hex]}));
                }
            }
        });
        row(ui, 28.0, |ui| {
            let hidden = sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.hides_contents()));
            if widgets::checkbox(ui, hidden, tr("Hide contents")) {
                app.run("section.toggleContents", json!({"ids": ids, "hidden": !hidden}));
            }
        });
    }
    widgets::sublabels(ui, tr("Status"), None);
    let changed = sel.iter().any(|i| doc.node(*i).is_some_and(|n| n.dev_status == DevStatus::ReadyForDev && n.dev_changed));
    row(ui, 32.0, |ui| status_dropdown(app, ui, doc, &ids, col * 2.0 + 8.0));
    if changed {
        // Changed since marked ready: say why to resolve it (back to Ready for dev).
        row(ui, 32.0, |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let id = egui::Id::new(("dev_reason", ids.first().copied()));
            let draft_id = id.with("draft");
            // The typed reason stays a draft until Enter or Resolve: clicking away keeps it.
            let mut draft: String = ui.data(|d| d.get_temp(draft_id)).unwrap_or_default();
            let committed = widgets::text_field(ui, id, &draft, col * 2.0 - 64.0, tr("Why it changed"));
            let enter = committed.is_some() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if let Some(text) = committed {
                draft = text;
                ui.data_mut(|d| d.insert_temp(draft_id, draft.clone()));
            }
            let button = widgets::button(ui, tr("Resolve"), true, true).on_hover_text(tr("Back to Ready for dev"));
            if enter || button.clicked() {
                let reason = draft.trim();
                let params = if reason.is_empty() { json!({"ids": ids}) } else { json!({"ids": ids, "reason": reason}) };
                app.run("dev.resolveChange", params);
                ui.data_mut(|d| d.remove::<String>(draft_id));
            }
        });
    } else if let Some(note) = common(doc, sel, |n| n.dev_note.clone()).filter(|n| !n.is_empty()) {
        row(ui, 24.0, |ui| {
            let t = crate::theme::Tokens::get(ui.ctx());
            let text = widgets::elide(ui, &trf("Last change: {note}", &[("note", &note)]), col * 2.0 + 8.0);
            widgets::label(ui, &text, t.text_secondary);
        });
    }
    row(ui, 32.0, |ui| {
        let fit = ui
            .add_sized(vec2(col * 2.0 + 8.0, widgets::FIELD_H), egui::Button::new(tr("Resize to fit")))
            .on_hover_text(crate::shortcuts::tip(tr("Fit to contents"), "object.resizeToFit"));
        if fit.clicked() {
            app.run("object.resizeToFit", json!({"ids": ids}));
        }
    });
    ui.add_space(4.0);
    widgets::separator(ui);
}
