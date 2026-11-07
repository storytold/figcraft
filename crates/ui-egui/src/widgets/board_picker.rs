//! Boards in the UI, continued: the toolbar's templates picker (a popover of the board templates,
//! each with a picture of itself, and a table) and the table's part of the bar over the selection
//! (add or remove a row or a column).

use std::collections::HashMap;

use egui::{Align2, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_engine::board_templates::TEMPLATES;
use serde_json::json;

use crate::FigApp;
use crate::a11y;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular};
use crate::toolbar::tool_cell;

/// The picker's key while it is open (`app.ui.tool_flyout`).
const KEY: &str = "board_templates";
const CARD: egui::Vec2 = vec2(136.0, 104.0);
const PICTURE_H: f32 = 76.0;
const COLUMNS: usize = 3;
const PAD: f32 = 10.0;
const GAP: f32 = 8.0;
const TITLE_H: f32 = 26.0;

/// The templates button at `at` (its left edge, the toolbar's middle) and, while open, its picker
/// above the toolbar (whose top is `top`). The button's right edge: the toolbar puts the next
/// button a GAP after it.
pub(crate) fn button(app: &mut FigApp, ui: &mut Ui, at: Pos2, top: f32, t: &Tokens) -> f32 {
    let r = Rect::from_min_size(pos2(at.x, at.y - 16.0), vec2(32.0, 32.0));
    let open = app.ui.tool_flyout == Some(KEY);
    let resp = tool_cell(ui, t, r, "board_templates", Icon::Templates, open, tr("Templates"));
    if open {
        a11y::popup(&mut app.ui.nav, resp.id, area_id());
    }
    if resp.clicked() {
        app.ui.tool_flyout = if open { None } else { Some(KEY) };
    }
    if app.ui.tool_flyout == Some(KEY) {
        picker(app, ui, r, top - 8.0, t);
    }
    r.max.x
}

fn area_id() -> egui::Id {
    egui::Id::unique("board_templates_area")
}

/// A template's picture: the template built on a board of its own and drawn small.
fn picture(ctx: &egui::Context, id: &str) -> Option<egui::TextureHandle> {
    let doc = figcraft_engine::board_templates::preview(id).ok()?;
    let layers = doc.node(doc.first_page()?)?.children.clone();
    let b = layers.iter().filter_map(|l| figcraft_render::export_bounds(&doc, *l)).reduce(|a, b| a.union(b))?;
    if !(b.width() > 0.0 && b.height() > 0.0) {
        return None;
    }
    let fit = (f64::from(CARD.x - 16.0) / b.width()).min(f64::from(PICTURE_H - 12.0) / b.height());
    let pix = figcraft_render::render_area(&doc, &layers, b, (fit * f64::from(ctx.pixels_per_point())).min(1.0))?;
    let image = crate::render_worker::screen_image(&pix, doc.color_profile);
    Some(ctx.load_texture(format!("board-template-{id}"), image, egui::TextureOptions::LINEAR))
}

/// The templates' pictures, drawn the first time the picker opens.
#[derive(Clone, Default)]
struct Pictures(HashMap<&'static str, Option<egui::TextureHandle>>);

/// The picker: a card for each template and one for a table, its bottom at `bottom`, from the
/// button `anchor`'s right edge leftwards (inside the canvas).
fn picker(app: &mut FigApp, ui: &mut Ui, anchor: Rect, bottom: f32, t: &Tokens) {
    let ctx = ui.ctx().clone();
    let mut pictures: Pictures = ctx.data(|d| d.get_temp(area_id())).unwrap_or_default();
    for tpl in TEMPLATES {
        pictures.0.entry(tpl.id).or_insert_with(|| picture(&ctx, tpl.id));
    }
    let count = TEMPLATES.len() + 1;
    let rows = count.div_ceil(COLUMNS);
    let size = vec2(COLUMNS as f32 * (CARD.x + GAP) - GAP + 2.0 * PAD, TITLE_H + rows as f32 * (CARD.y + GAP) - GAP + 2.0 * PAD);
    let canvas = app.canvas.rect;
    let x = (anchor.max.x - size.x).min(canvas.max.x - size.x - 8.0).max(canvas.min.x + 8.0);
    let mut chosen = None;
    egui::Area::new(area_id()).fixed_pos(pos2(x, bottom - size.y)).order(egui::Order::Foreground).show(&ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        a11y::container_bounds(ui, rect);
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(13), t.menu);
        p.text(pos2(rect.min.x + PAD + 2.0, rect.min.y + PAD + 8.0), Align2::LEFT_CENTER, tr("Templates"), crate::theme::semibold(12.0), t.menu_text);
        for i in 0..count {
            let (col, row) = (i % COLUMNS, i / COLUMNS);
            let at = pos2(rect.min.x + PAD + col as f32 * (CARD.x + GAP), rect.min.y + PAD + TITLE_H + row as f32 * (CARD.y + GAP));
            let r = Rect::from_min_size(at, CARD);
            let tpl = TEMPLATES.get(i);
            let label = tpl.map_or(tr("Table"), |tpl| tr(tpl.name));
            let resp = ui.interact(r, ui.scope_id().with(("board_template", i)), Sense::click());
            a11y::menu_item(&resp, label, "", None);
            a11y::own_focus(&resp);
            let p = ui.painter();
            if resp.hovered() || resp.has_focus() {
                p.rect_filled(r, CornerRadius::same(8), t.menu_hover);
            }
            let pic = Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(CARD.x - 8.0, PICTURE_H));
            p.rect_filled(pic, CornerRadius::same(6), crate::theme::c32(figcraft_doc::BOARD_PAGE));
            p.rect_stroke(pic, CornerRadius::same(6), Stroke::new(1.0, t.border), StrokeKind::Inside);
            match tpl.and_then(|tpl| pictures.0.get(tpl.id)).and_then(Option::as_ref) {
                Some(tex) => {
                    let s = tex.size_vec2() / ctx.pixels_per_point();
                    let k = ((pic.width() - 12.0) / s.x).min((pic.height() - 12.0) / s.y).min(1.0);
                    p.image(
                        tex.id(),
                        Rect::from_center_size(pic.center(), s * k),
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                // The table (and a template whose picture couldn't be drawn): its mark.
                None => icons::paint(
                    p,
                    Rect::from_center_size(pic.center(), vec2(32.0, 32.0)),
                    if tpl.is_some() { Icon::Templates } else { Icon::Table },
                    egui::Color32::from_gray(90),
                ),
            }
            p.text(pos2(r.min.x + 6.0, pic.max.y + 13.0), Align2::LEFT_CENTER, label, regular(12.0), t.menu_text);
            if resp.clicked() {
                chosen = Some(i);
            }
        }
    });
    ctx.data_mut(|d| d.insert_temp(area_id(), pictures));
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.tool_flyout = None;
    }
    if let Some(i) = chosen {
        let (cmd, params) = match TEMPLATES.get(i) {
            Some(tpl) => ("board.template", json!({"name": tpl.id})),
            None => ("board.table", json!({})),
        };
        app.run(cmd, params);
        app.ui.tool_flyout = None;
    }
}

/// The table's part of the bar over the selection from `x`: add a row or a column at the end,
/// remove the last row or column. Where the next part starts.
pub(crate) fn table_part(app: &mut FigApp, ui: &mut Ui, mut x: f32, cy: f32, t: &Tokens) -> f32 {
    let cells = [
        (Icon::AddRow, tr("Add row"), "addRow"),
        (Icon::RemoveRow, tr("Remove row"), "removeRow"),
        (Icon::AddColumn, tr("Add column"), "addColumn"),
        (Icon::RemoveColumn, tr("Remove column"), "removeColumn"),
    ];
    for (icon, tip, action) in cells {
        let r = Rect::from_center_size(pos2(x + 14.0, cy), vec2(28.0, 28.0));
        if tool_cell(ui, t, r, ("bb_table", action), icon, false, tip).clicked() {
            app.run("board.tableEdit", json!({"action": action}));
        }
        x += 30.0;
    }
    crate::board_bar::divider(ui, t, x, cy)
}

/// The table part's width (with the gap after it).
pub(crate) const TABLE_PART_W: f32 = 4.0 * 30.0 + 8.0;
