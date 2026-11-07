//! The Frame tool's panel: New from selection, then frame presets by category. Clicking a preset
//! adds a frame of that size next to the page's frames; either returns to the Move tool.

use egui::{Align2, CornerRadius, Id, Sense, Ui, pos2, vec2};
use figcraft_doc::{Document, NodeId, NodeKind};
use serde_json::json;

use crate::FigApp;
use crate::a11y;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{self, PAD};

/// Whether `object.frameSelection` would frame `selection`: layers on a page, none of them a
/// section (sections never go in frames, #1683).
fn can_frame(doc: &Document, selection: &[NodeId]) -> bool {
    !selection.is_empty() && selection.iter().all(|i| doc.node(*i).is_some_and(|n| n.parent.is_some() && !matches!(n.kind, NodeKind::Section { .. })))
}

/// Draw New from selection and the expandable frame-preset categories. New from selection wraps
/// the selected layers in a frame (`object.frameSelection`, #1683); choosing a preset runs
/// `frame.create` and zooms to the new selection. Both switch to the Move tool.
pub fn panel(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    widgets::section_header(ui, tr("Frame"), &[], false);
    let framable = app.session.ds().is_ok_and(|ds| can_frame(&ds.doc, &ds.selection));
    let mut from_selection = false;
    ui.horizontal(|ui| {
        ui.add_space(PAD);
        let resp = widgets::button(ui, tr("New from selection"), false, framable);
        if !framable {
            a11y::disabled(&resp);
        }
        from_selection = resp.clicked();
    });
    ui.add_space(4.0);
    // The Frame tool stays when framing failed (its error is shown).
    if from_selection && app.run("object.frameSelection", json!({})).is_some() {
        app.run("tool.set", json!({"tool": "move"}));
        return;
    }
    let w = ui.available_width();
    let mut cats: Vec<&str> = Vec::new();
    for (c, ..) in figcraft_engine::cmd::create::FRAME_PRESETS {
        if !cats.contains(c) {
            cats.push(c);
        }
    }
    let mut chosen = None;
    for cat in cats {
        let open_id = Id::unique(("preset_cat", cat));
        // The first category starts open.
        let open: bool = ui.data(|d| d.get_temp(open_id)).unwrap_or(cat == "Phone");
        let (r, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r.shrink2(vec2(8.0, 2.0)), CornerRadius::same(5), t.hover);
        }
        icons::paint(
            ui.painter(),
            egui::Rect::from_center_size(pos2(r.min.x + PAD + 4.0, r.center().y), vec2(10.0, 10.0)),
            if open { Icon::Chevron } else { Icon::ChevronRight },
            t.icon_secondary,
        );
        ui.painter().text(pos2(r.min.x + PAD + 16.0, r.center().y), Align2::LEFT_CENTER, tr(cat), semibold(11.0), t.text);
        if resp.clicked() {
            ui.data_mut(|d| d.insert_temp(open_id, !open));
        }
        if !open {
            continue;
        }
        for (_, name, pw, ph) in figcraft_engine::cmd::create::FRAME_PRESETS.iter().filter(|(c, ..)| *c == cat) {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r.shrink2(vec2(8.0, 1.0)), CornerRadius::same(5), t.hover);
            }
            ui.painter().text(pos2(r.min.x + PAD + 16.0, r.center().y), Align2::LEFT_CENTER, *name, regular(11.0), t.text);
            ui.painter().text(
                pos2(r.max.x - PAD, r.center().y),
                Align2::RIGHT_CENTER,
                format!("{}×{}", crate::canvas::fmt(*pw), crate::canvas::fmt(*ph)),
                regular(11.0),
                t.text_secondary,
            );
            if resp.clicked() {
                chosen = Some(*name);
            }
        }
    }
    if let Some(name) = chosen {
        app.run("frame.create", json!({"preset": name}));
        app.run("tool.set", json!({"tool": "move"}));
        // Show the new frame.
        app.run("view.zoomSelection", json!({}));
    }
}
