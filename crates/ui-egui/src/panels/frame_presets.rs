//! The Frame tool's panel: frame presets by category. Clicking one adds a frame of that size next
//! to the page's frames and returns to the Move tool.

use egui::{Align2, CornerRadius, Id, Sense, Ui, pos2, vec2};
use serde_json::json;

use crate::FigApp;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{self, PAD};

/// Draw the expandable frame-preset categories; choosing a preset runs `frame.create`,
/// switches to the Move tool and zooms to the new selection.
pub fn panel(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    widgets::section_header(ui, tr("Frame"), &[], false);
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
