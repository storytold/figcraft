//! The small bar floating over a selected plugin widget that has a property menu: its actions and
//! toggles as buttons, its colour choices as swatches. Each calls `widget.menu`; the labels are the
//! plugin's own.

use egui::{CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_engine::widgets::{MenuItem, MenuKind, menu_of};
use serde_json::json;

use crate::FigApp;
use crate::a11y;
use crate::board_bar::{Mark, divider, paint_choice, place};
use crate::theme::{Tokens, semibold};

const HEIGHT: f32 = 40.0;
const CELL: f32 = 28.0;
const SWATCH: f32 = 20.0;
const GAP: f32 = 4.0;
/// A text button's padding on each side.
const PAD: f32 = 8.0;

/// What one item takes across the bar, its trailing gap included (text buttons by their label).
fn width(ctx: &egui::Context, m: &MenuItem) -> f32 {
    match m.kind {
        MenuKind::Color => m.options.len() as f32 * (SWATCH + GAP),
        _ => ctx.fonts_mut(|f| f.layout_no_wrap(m.label.clone(), semibold(11.0), egui::Color32::WHITE).size().x) + PAD * 2.0 + GAP,
    }
}

/// The bar above the selected widget, while it is the only layer selected, nothing is being dragged
/// or edited and the file can be edited.
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    if app.session.tool.busy() || app.session.active_read_only().is_some() {
        return;
    }
    let Ok(ds) = app.session.ds() else { return };
    let ([w], None) = (ds.selection.as_slice(), &ds.text_edit) else { return };
    let w = *w;
    let menu = menu_of(&ds.doc, w);
    let Some(b) = ds.doc.world_bounds(w).filter(|_| !menu.is_empty()) else { return };
    let sel = Rect::from_two_pos(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1));
    // Scrolled out of view: no bar (it would float over the rest of the window).
    if !sel.intersects(app.canvas.rect) {
        return;
    }
    let widths: Vec<f32> = menu.iter().map(|m| width(ctx, m)).collect();
    // The items, a divider (8) between each two, and the padding.
    let size = vec2(8.0 + widths.iter().sum::<f32>() + 8.0 * (menu.len() - 1) as f32 + 4.0, HEIGHT);
    let at = place(sel, size, app.canvas.rect);
    let t = Tokens::get(ctx);
    egui::Area::new(egui::Id::unique("widget_bar")).fixed_pos(at).order(egui::Order::Middle).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(size, Sense::click());
        a11y::container_bounds(ui, rect);
        let p = ui.painter();
        p.rect_filled(rect.expand(2.0).translate(vec2(0.0, 1.0)), CornerRadius::same(12), egui::Color32::from_black_alpha(18));
        p.rect_filled(rect, CornerRadius::same(10), t.panel);
        p.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, t.border), StrokeKind::Inside);
        let cy = rect.center().y;
        let mut x = rect.min.x + 8.0;
        for (i, (m, wd)) in menu.iter().zip(&widths).enumerate() {
            if i > 0 {
                x = divider(ui, &t, x, cy);
            }
            if let Some(value) = item(ui, &t, m, x, cy, *wd) {
                app.run("widget.menu", json!({"id": w, "item": m.item, "value": value}));
            }
            x += wd;
        }
    });
}

/// Draw item `m` from `x`, `wd` wide; the value to send when it's used.
fn item(ui: &mut Ui, t: &Tokens, m: &MenuItem, x: f32, cy: f32, wd: f32) -> Option<serde_json::Value> {
    if m.kind == MenuKind::Color {
        let mut picked = None;
        for (i, hex) in m.options.iter().enumerate() {
            let r = Rect::from_center_size(pos2(x + i as f32 * (SWATCH + GAP) + SWATCH / 2.0, cy), vec2(SWATCH, SWATCH));
            let tip = format!("{} {hex}", m.label);
            let resp = ui.interact(r, ui.scope_id().with(("wb_color", &m.item, i)), Sense::click()).on_hover_text(&tip);
            let on = m.value.as_ref() == Some(hex);
            a11y::toggle(&resp, &tip, on);
            if on {
                ui.painter().rect_stroke(r.expand(2.5), CornerRadius::same(6), Stroke::new(1.5, t.text), StrokeKind::Inside);
            } else if resp.hovered() {
                ui.painter().rect_filled(r.expand(2.0), CornerRadius::same(6), t.hover);
            }
            if let Some(c) = figcraft_color::Color::from_hex(hex) {
                paint_choice(ui.painter(), r, &Mark::Color(c), t.icon);
            }
            if resp.clicked() && !on {
                picked = Some(json!(hex));
            }
        }
        return picked;
    }
    let r = Rect::from_min_size(pos2(x, cy - CELL / 2.0), vec2(wd - GAP, CELL));
    let resp = ui.interact(r, ui.scope_id().with(("wb_item", &m.item)), Sense::click());
    let on = m.kind == MenuKind::Toggle && m.checked;
    if m.kind == MenuKind::Toggle {
        a11y::toggle(&resp, &m.label, on);
    } else {
        a11y::button(&resp, &m.label);
    }
    let bg = if on {
        t.accent
    } else if resp.hovered() {
        t.hover
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(r, CornerRadius::same(5), bg);
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, &m.label, semibold(11.0), if on { t.accent_on } else { t.text });
    resp.clicked().then(|| if m.kind == MenuKind::Toggle { json!(!m.checked) } else { serde_json::Value::Null })
}
