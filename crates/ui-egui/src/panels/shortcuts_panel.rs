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

/// A keycap's left edge: LTR keycaps grow left; RTL keycaps grow right.
fn keycap_left(anchor: f32, width: f32, rtl: bool) -> f32 {
    if rtl { anchor } else { anchor - width }
}

/// Paint `text` as a keycap at the reading-side edge, returning the next edge.
fn keycap(ui: &Ui, t: &Tokens, text: &str, edge: f32, cy: f32, rtl: bool) -> f32 {
    let galley = ui.painter().layout_no_wrap(text.to_string(), regular(11.0), t.text);
    let w = galley.size().x + 10.0;
    let r = egui::Rect::from_min_size(pos2(keycap_left(edge, w, rtl), cy - 10.0), vec2(w, 20.0));
    ui.painter().rect_filled(r, CornerRadius::same(4), t.field);
    ui.painter().rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, t.border), StrokeKind::Inside);
    ui.painter().galley(pos2(r.min.x + 5.0, cy - galley.size().y / 2.0), galley, t.text);
    if rtl { r.max.x } else { r.min.x }
}

fn row(ui: &mut Ui, t: &Tokens, row: &Row, width: f32) {
    let (r, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(4), t.hover);
    }
    let cy = r.center().y;
    let rtl = i18n::right_to_left();
    let mut edge = if rtl { r.min.x + 6.0 } else { r.max.x - 6.0 };
    for (i, k) in row.keys.iter().enumerate().rev() {
        edge = keycap(ui, t, k, edge, cy, rtl);
        if i > 0 {
            let or = ui.painter().layout_no_wrap(tr("or").into(), regular(10.0), t.text_tertiary);
            if rtl {
                edge += 4.0;
                let or_width = or.size().x;
                let or_height = or.size().y;
                ui.painter().galley(pos2(edge, cy - or_height / 2.0), or, t.text_tertiary);
                edge += or_width + 4.0;
            } else {
                edge -= 4.0 + or.size().x;
                ui.painter().galley(pos2(edge, cy - or.size().y / 2.0), or, t.text_tertiary);
                edge -= 4.0;
            }
        }
    }
    let label_w = (if rtl { r.max.x - edge - 14.0 } else { edge - r.min.x - 14.0 }).max(20.0);
    // Elide against the free space between translated label and keycaps; keep the full hover tip.
    let label = crate::widgets::elide_font(ui, &row.label, label_w, regular(12.0));
    if label != row.label {
        resp.on_hover_text(&*row.label);
    }
    let anchor = if rtl { r.max.x - 6.0 } else { r.min.x + 6.0 };
    let align = if rtl { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER };
    ui.painter().text(pos2(anchor, cy), align, label, regular(12.0), t.text);
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
        // The first logical shortcut group starts in the rightmost column in Hebrew.
        let mut groups: Vec<_> = list.chunks(per.max(1)).collect();
        if i18n::right_to_left() {
            groups.reverse();
        }
        egui::ScrollArea::vertical().min_scrolled_height(body_h).max_height(body_h).show(ui, |ui| {
            ui.set_min_height(body_h);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = vec2(gap, 0.0);
                for chunk in groups {
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
#[cfg(test)]
mod rtl_tests {
    use super::keycap_left;

    #[test]
    fn hebrew_keycaps_use_opposite_reading_edge() {
        assert_eq!(keycap_left(100.0, 30.0, false), 70.0);
        assert_eq!(keycap_left(100.0, 30.0, true), 100.0);
    }
}
