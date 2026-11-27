//! The ink bar: while the Pencil, Marker or Highlighter is the tool, its colour and stroke weight
//! float above the bottom toolbar, so they are chosen before drawing (#2395; `tool.ink`). Each
//! tool keeps its own for the session.

use egui::{CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use figcraft_engine::ToolId;
use figcraft_engine::board::{INK_COLORS, INK_WEIGHTS};
use serde_json::json;

use crate::FigApp;
use crate::a11y::{self, Region};
use crate::board_bar::{GAP, Mark, SWATCH, bar, color_label, divider, paint_choice, paint_wheel};
use crate::colorpicker::PickerTarget;
use crate::i18n::tr;
use crate::icons::Icon;
use crate::theme::Tokens;
use crate::widgets::{self, Lead};

/// The stroke weight field's width.
const FIELD_W: f32 = 72.0;
const HEIGHT: f32 = 40.0;

/// The ink bar above the toolbar pill at `pill`, while the tool draws ink and no flyout (which
/// opens where the bar sits) is open.
pub(crate) fn show(app: &mut FigApp, ui: &Ui, pill: Rect) {
    let tool = app.session.tool.tool;
    let Some(ink) = app.session.tool.ink(tool) else { return };
    if app.ui.tool_flyout.is_some() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    // The Pencil's own colour (black, or white on a dark page or frame) comes first.
    let auto = tool == ToolId::Pencil;
    let chips = INK_COLORS.len() + 1 + usize::from(auto);
    let size = vec2(8.0 + chips as f32 * (SWATCH + GAP) - GAP + 8.0 + FIELD_W + 8.0, HEIGHT);
    let at = pos2(pill.center().x - size.x / 2.0, pill.min.y - 8.0 - HEIGHT);
    egui::Area::new(Id::unique("ink_bar")).fixed_pos(at).order(egui::Order::Middle).show(ui.ctx(), |ui| {
        let rect = bar(ui, size, &t);
        a11y::area(&mut app.ui.nav, Region::Toolbar, rect);
        let cy = rect.center().y;
        let mut x = rect.min.x + 8.0;
        // One swatch from `x`: hovered it lifts, the ink's own is ringed.
        let mut chip = |ui: &mut Ui, key: usize, label: &str, on: bool| {
            let r = Rect::from_center_size(pos2(x + SWATCH / 2.0, cy), vec2(SWATCH, SWATCH));
            let resp = ui.interact(r, ui.scope_id().with(("ink", key)), Sense::click()).on_hover_text(label);
            a11y::toggle(&resp, label, on);
            if on {
                ui.painter().rect_stroke(r.expand(3.0), CornerRadius::same(7), Stroke::new(2.0, t.accent), StrokeKind::Inside);
            } else if resp.hovered() {
                ui.painter().rect_filled(r.expand(2.0), CornerRadius::same(6), t.hover);
            }
            x += SWATCH + GAP;
            (resp, r)
        };
        let mut chosen = None;
        if auto {
            let (resp, r) = chip(ui, 0, tr("Auto: black, or white on a dark background"), ink.color.is_none());
            let (black, white) = (crate::theme::c32(figcraft_engine::tools::PENCIL_STROKE.0), egui::Color32::WHITE);
            let p = ui.painter();
            p.rect_filled(r, CornerRadius::same(4), white);
            p.rect_filled(Rect::from_min_max(r.min, pos2(r.center().x, r.max.y)), CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }, black);
            p.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, t.border), StrokeKind::Inside);
            if resp.clicked() {
                chosen = Some("auto");
            }
        }
        for (i, (name, c)) in INK_COLORS.iter().enumerate() {
            let (resp, r) = chip(ui, i + 1, tr(color_label(name)), ink.color == Some(*c));
            paint_choice(ui.painter(), r, &Mark::Color(*c), t.icon);
            // Outlined so a dark ink still reads against the dark bar.
            ui.painter().rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, t.border), StrokeKind::Inside);
            if resp.clicked() {
                chosen = Some(*name);
            }
        }
        let custom = ink.color.is_some_and(|c| !INK_COLORS.iter().any(|(_, k)| *k == c));
        let (resp, r) = chip(ui, INK_COLORS.len() + 1, tr("Edit colour"), custom);
        paint_wheel(ui.painter(), r.center(), SWATCH / 2.0 - 1.0);
        app.picker_control(PickerTarget::Ink { tool }, &resp);
        if let Some(color) = chosen {
            app.run("tool.ink", json!({"tool": tool.id(), "color": color}));
        }
        let x = divider(ui, &t, x - GAP, cy);
        let field = Rect::from_min_size(pos2(x, cy - 12.0), vec2(FIELD_W, 24.0));
        let edit = ui
            .scope_builder(UiBuilder::new().max_rect(field), |ui| {
                widgets::number_field(
                    ui,
                    Id::unique(("ink_weight", tool.id())),
                    Lead::Icon(Icon::Menu, tr("Stroke weight")),
                    Some(ink.weight),
                    "",
                    FIELD_W,
                    1.0,
                )
            })
            .inner;
        let (lo, hi) = (*INK_WEIGHTS.start(), *INK_WEIGHTS.end());
        app.field_run(edit, "tool.ink", |v| json!({"tool": tool.id(), "weight": v.clamp(lo, hi)}));
    });
}
