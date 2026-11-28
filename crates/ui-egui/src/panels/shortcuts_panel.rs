//! Help ▸ Keyboard shortcuts: a panel along the bottom of the window with a tab per group of the
//! keymap (Essential, Tools, View…), each command's keys as keycaps for this platform, and a search
//! across every group.

use egui::{Align2, Context, CornerRadius, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_engine::keymap::{GESTURES, Group, KEYS, Platform};

use crate::FigApp;
use crate::i18n::{self, tr};
use crate::theme::{Tokens, regular};

/// Most keys a row shows (a command's first bindings; the rest stay in the menus' alternates).
const MAX_KEYS: usize = 2;
const ROW_H: f32 = 26.0;
const COLUMNS: usize = 3;
/// Rows a column shows before the panel scrolls.
const MAX_ROWS: usize = 9;

/// One row: the action and its keys as written on this platform.
struct Row {
    label: std::borrow::Cow<'static, str>,
    keys: Vec<String>,
}

/// Rows per column of the fullest tab, so switching tabs doesn't resize the panel.
fn tallest_column() -> usize {
    let count = |g: Group| KEYS.iter().filter(|b| b.group == g).count() + GESTURES.iter().filter(|x| x.0 == g).count();
    Group::ALL.iter().map(|g| count(*g).div_ceil(COLUMNS)).max().unwrap_or(1)
}

fn rows(group: Option<Group>, query: &str, platform: Platform) -> Vec<Row> {
    let q = query.trim().to_lowercase();
    // Labels match in the UI language and in English.
    let wanted = |g: Group, label: &str, english: &str| {
        group.is_none_or(|x| x == g) && (q.is_empty() || label.to_lowercase().contains(&q) || english.to_lowercase().contains(&q))
    };
    let mut out: Vec<Row> = KEYS
        .iter()
        .map(|b| (b, i18n::command(b.id, b.title())))
        .filter(|(b, label)| wanted(b.group, label, b.title()))
        .map(|(b, label)| {
            // Alternatives that read the same (⌘+ and ⇧⌘+) show once.
            let mut keys: Vec<String> = Vec::with_capacity(MAX_KEYS);
            for k in b.keys(platform) {
                let shown = figcraft_engine::keymap::display(k, platform);
                if keys.len() < MAX_KEYS && !keys.contains(&shown) {
                    keys.push(shown);
                }
            }
            Row { label, keys }
        })
        .collect();
    out.extend(GESTURES.iter().filter(|(g, label, ..)| wanted(*g, tr(label), label)).map(|(_, label, mac, other)| Row {
        label: tr(label).into(),
        keys: vec![tr(if platform == Platform::Mac { mac } else { other }).to_string()],
    }));
    out
}

/// Paint `text` as a keycap ending at `right`; returns its left edge.
fn keycap(ui: &Ui, t: &Tokens, text: &str, right: f32, cy: f32) -> f32 {
    let galley = ui.painter().layout_no_wrap(text.to_string(), regular(11.0), t.text);
    let w = galley.size().x + 10.0;
    let r = egui::Rect::from_min_size(pos2(right - w, cy - 10.0), vec2(w, 20.0));
    ui.painter().rect_filled(r, CornerRadius::same(4), t.field);
    ui.painter().rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, t.border), StrokeKind::Inside);
    ui.painter().galley(pos2(r.min.x + 5.0, cy - galley.size().y / 2.0), galley, t.text);
    r.min.x
}

fn row(ui: &mut Ui, t: &Tokens, row: &Row, width: f32) {
    let (r, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(4), t.hover);
    }
    let cy = r.center().y;
    let mut right = r.max.x - 6.0;
    for (i, k) in row.keys.iter().enumerate().rev() {
        right = keycap(ui, t, k, right, cy);
        if i > 0 {
            let or = ui.painter().layout_no_wrap(tr("or").into(), regular(10.0), t.text_tertiary);
            right -= 4.0 + or.size().x;
            ui.painter().galley(pos2(right, cy - or.size().y / 2.0), or, t.text_tertiary);
            right -= 4.0;
        }
    }
    let label_w = (right - r.min.x - 14.0).max(20.0);
    // Measured in the font it's painted in: a long translation ends with "…" before the keycaps and
    // shows in full on hover.
    let label = crate::widgets::elide_font(ui, &row.label, label_w, regular(12.0));
    if label != row.label {
        resp.on_hover_text(&*row.label);
    }
    ui.painter().text(pos2(r.min.x + 6.0, cy), Align2::LEFT_CENTER, label, regular(12.0), t.text);
}

pub fn show(app: &mut FigApp, ctx: &Context) {
    if !app.ui.shortcuts_panel {
        return;
    }
    let t = Tokens::get(ctx);
    let platform = crate::shortcuts::platform();
    let width = (ctx.content_rect().width() - 48.0).clamp(480.0, 1040.0);
    let mut open = true;
    crate::dialogs::window(tr("Keyboard shortcuts"), ctx).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -16.0)).open(&mut open).show(ctx, |ui| {
        ui.set_width(width);
        ui.horizontal(|ui| {
            crate::dialogs::tab_strip(ui, &t, &Group::ALL, |g| tr(g.label()), &mut app.ui.shortcuts_tab);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut app.ui.shortcuts_query).hint_text(tr("Search shortcuts")).desired_width(180.0));
                crate::widgets::select_all_on_focus(ui, &r, &app.ui.shortcuts_query);
            });
        });
        ui.add_space(10.0);
        let searching = !app.ui.shortcuts_query.trim().is_empty();
        let list = rows((!searching).then_some(app.ui.shortcuts_tab), &app.ui.shortcuts_query, platform);
        let body_h = (ROW_H + 2.0) * tallest_column().min(MAX_ROWS) as f32;
        if list.is_empty() {
            ui.allocate_ui(vec2(width, body_h), |ui| {
                ui.set_min_height(body_h);
                ui.label(egui::RichText::new(tr("No shortcuts match")).color(t.text_secondary).font(regular(12.0)));
            });
            return;
        }
        let gap = 24.0;
        let col_w = (width - gap * (COLUMNS as f32 - 1.0)) / COLUMNS as f32;
        let per = list.len().div_ceil(COLUMNS);
        egui::ScrollArea::vertical().min_scrolled_height(body_h).max_height(body_h).show(ui, |ui| {
            ui.set_min_height(body_h);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = vec2(gap, 0.0);
                for chunk in list.chunks(per.max(1)) {
                    ui.vertical(|ui| {
                        ui.set_width(col_w);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for r in chunk {
                            row(ui, &t, r, col_w);
                        }
                    });
                }
            });
        });
    });
    if !open {
        app.ui.shortcuts_panel = false;
    }
}
