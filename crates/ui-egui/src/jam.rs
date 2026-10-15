//! CraftJam boards: the whiteboard interface a board file shows instead of the design panels.
//!
//! No side panels: the file's name and the main menu float at the top left, the zoom at the
//! bottom right, and the bottom toolbar has the board's tools (select and hand, the pens, sticky
//! notes, shapes and the connector, text, sections, tables, and the "+" inserts). What's selected
//! is edited in the context toolbar floating above it (`jam_bar`).

use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_engine::ToolId;
use serde_json::json;

use crate::FigApp;
use crate::a11y::{self, Region, Tip};
use crate::i18n::{tr, tr_noop};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};
use crate::toolbar::{divider, pill, pretty_shortcut, tool_cell};

/// Whether the file being edited is a board.
pub fn is_board(app: &FigApp) -> bool {
    app.session.doc().is_ok_and(|d| d.board)
}

/// A floating card (the board's chrome): white, rounded, with a soft shadow.
fn card(ui: &Ui, rect: Rect, t: &Tokens) {
    let painter = ui.painter();
    for (i, a) in [(4.0, 10u8), (2.0, 16), (1.0, 22)] {
        painter.rect_filled(rect.expand(i).translate(vec2(0.0, i * 0.5)), CornerRadius::same((10.0 + i) as u8), Color32::from_black_alpha(a));
    }
    painter.rect_filled(rect, CornerRadius::same(10), t.panel);
    painter.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, t.border), StrokeKind::Inside);
}

/// The file card at the top left (main menu, the file's name, "CraftJam") and the zoom at the
/// bottom right.
pub fn chrome(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let canvas = app.canvas.rect;
    // File card.
    let name = app.session.doc().map(|d| d.name.clone()).unwrap_or_default();
    let name_w = ui.painter().layout_no_wrap(name.clone(), semibold(12.5), t.text).size().x.min(240.0);
    let badge = tr("CraftJam");
    let badge_w = ui.painter().layout_no_wrap(badge.to_string(), regular(11.0), t.text_secondary).size().x;
    let rect = Rect::from_min_size(canvas.min + vec2(12.0, 12.0), vec2(44.0 + name_w + 14.0 + badge_w + 16.0, 40.0));
    a11y::area(&mut app.ui.nav, Region::Layers, rect);
    card(ui, rect, &t);
    let mr = Rect::from_center_size(pos2(rect.min.x + 24.0, rect.center().y), vec2(32.0, 30.0));
    let mresp = ui.interact(mr, ui.id().with("jam_menu"), Sense::click()).tip(tr("Main menu"));
    if mresp.hovered() || app.ui.main_menu {
        ui.painter().rect_filled(mr, CornerRadius::same(6), t.hover);
    }
    icons::paint(ui.painter(), Rect::from_center_size(mr.center(), vec2(18.0, 18.0)), Icon::Logo, t.icon);
    if mresp.clicked() {
        app.ui.main_menu = !app.ui.main_menu;
    }
    if app.ui.main_menu {
        a11y::popup(&mut app.ui.nav, mresp.id, crate::menus::main_menu_id());
    }
    let shown = crate::widgets::elide_font(ui, &name, 240.0, semibold(12.5));
    ui.painter().text(pos2(rect.min.x + 44.0, rect.center().y), Align2::LEFT_CENTER, shown, semibold(12.5), t.text);
    let br = Rect::from_min_size(pos2(rect.min.x + 44.0 + name_w + 10.0, rect.center().y - 10.0), vec2(badge_w + 12.0, 20.0));
    ui.painter().rect_filled(br, CornerRadius::same(10), t.hover);
    ui.painter().text(br.center(), Align2::CENTER_CENTER, badge, regular(11.0), t.text_secondary);
    // Zoom.
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
    let zr = Rect::from_min_size(pos2(canvas.max.x - 12.0 - 132.0, canvas.max.y - 12.0 - 40.0), vec2(132.0, 40.0));
    card(ui, zr, &t);
    let mut x = zr.min.x + 4.0;
    for (icon, tip, cmd, w) in [
        (Icon::Minus, tr("Zoom out"), "view.zoomOut", 32.0),
        (Icon::Search, tr("Zoom to fit"), "view.zoomFit", 60.0),
        (Icon::Plus, tr("Zoom in"), "view.zoomIn", 32.0),
    ] {
        let r = Rect::from_min_size(pos2(x, zr.min.y + 4.0), vec2(w, 32.0));
        let resp = ui.interact(r, ui.id().with(("jam_zoom", cmd)), Sense::click()).tip(tip);
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(6), t.hover);
        }
        if cmd == "view.zoomFit" {
            ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("{:.0}%", zoom * 100.0), regular(12.0), t.text);
        } else {
            icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(14.0, 14.0)), icon, t.icon);
        }
        if resp.clicked() {
            app.run(cmd, json!({}));
        }
        x += w;
    }
}

/// A board tool: what the button shows and does.
struct JamTool {
    tool: ToolId,
    icon: Icon,
    /// `tool.set` extras (the polygon tool's points).
    points: Option<u32>,
    label: &'static str,
    key: &'static str,
}

const fn jt(tool: ToolId, icon: Icon, points: Option<u32>, label: &'static str, key: &'static str) -> JamTool {
    JamTool { tool, icon, points, label, key }
}

/// The board's tools, in groups (a divider between groups).
fn groups() -> Vec<Vec<JamTool>> {
    vec![
        vec![jt(ToolId::Move, Icon::Move, None, tr_noop("Select"), "V"), jt(ToolId::Hand, Icon::Hand, None, tr_noop("Hand"), "H")],
        vec![jt(ToolId::Pencil, Icon::Marker, None, tr_noop("Marker"), "Shift+P")],
        vec![jt(ToolId::Sticky, Icon::Sticky, None, tr_noop("Sticky note"), "S")],
        vec![
            jt(ToolId::Rectangle, Icon::Rectangle, None, tr_noop("Square"), "R"),
            jt(ToolId::Ellipse, Icon::Ellipse, None, tr_noop("Ellipse"), "O"),
            jt(ToolId::Polygon, Icon::Diamond, Some(4), tr_noop("Diamond"), ""),
            jt(ToolId::Polygon, Icon::Polygon, Some(3), tr_noop("Triangle"), ""),
            jt(ToolId::Connector, Icon::Connector, None, tr_noop("Connector"), "X"),
        ],
        vec![jt(ToolId::Text, Icon::Text, None, tr_noop("Text"), "T"), jt(ToolId::Section, Icon::Section, None, tr_noop("Section"), "Shift+S")],
    ]
}

/// The board's bottom toolbar.
pub fn toolbar(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let canvas = app.canvas.rect;
    let groups = groups();
    let cell = 36.0;
    let count: usize = groups.iter().map(Vec::len).sum();
    // (One more cell: the library shapes' button.)
    let width = (count + 1) as f32 * (cell + 4.0) + groups.len() as f32 * 12.0 + 12.0;
    let rect = pill(ui, &mut app.ui.nav, canvas, width, &t);
    let current = app.session.tool.tool;
    let points = app.session.tool.polygon_points.unwrap_or(3);
    let mut x = rect.min.x + 8.0;
    let cy = rect.center().y;
    let preset = app.session.tool.shape_preset;
    for (gi, g) in groups.iter().enumerate() {
        if gi > 0 {
            divider(ui, &t, x + 2.0, cy);
            x += 12.0;
        }
        for jt in g {
            // The library shapes' button, before the connector.
            if jt.tool == ToolId::Connector {
                let r = Rect::from_center_size(pos2(x + cell / 2.0, cy), vec2(cell, cell));
                let resp = tool_cell(ui, &t, r, "jam_more_shapes", Icon::Shapes, preset.is_some() && current == ToolId::Rectangle, tr("More shapes"));
                if resp.clicked() {
                    let open = ui.data(|d| d.get_temp::<bool>(egui::Id::new("jam_more_shapes"))).unwrap_or(false);
                    ui.data_mut(|d| d.insert_temp(egui::Id::new("jam_more_shapes"), !open));
                }
                if ui.data(|d| d.get_temp::<bool>(egui::Id::new("jam_more_shapes"))).unwrap_or(false) {
                    more_shapes(app, ui.ctx(), pos2(r.min.x, rect.min.y - 8.0), resp.rect);
                }
                x += cell + 4.0;
            }
            // The pile of sticky notes: the next one's colour, and a click on it again picks another.
            if jt.tool == ToolId::Sticky {
                let r = Rect::from_center_size(pos2(x + cell / 2.0, cy), vec2(cell, cell));
                let i = app.session.tool.sticky_color;
                let color = figcraft_engine::board::STICKY_COLORS.get(i).map(|c| c.color).unwrap_or(figcraft_engine::board::STICKY_COLORS[0].color);
                let on = current == ToolId::Sticky;
                let resp = ui.interact(r, ui.id().with("jam_sticky"), Sense::click()).tip(format!("{}  {}", tr("Sticky note"), pretty_shortcut("S")));
                a11y::toggle(&resp, tr("Sticky note"), on);
                if on || resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
                }
                let [cr, cg, cb, _] = color.to_rgba8();
                for (k, dy) in [(2.0, 3.0), (1.0, 1.5), (0.0, 0.0)] {
                    let note = Rect::from_center_size(r.center() + vec2(k, -dy + 2.0), vec2(20.0, 20.0));
                    ui.painter().rect_filled(
                        note,
                        CornerRadius::same(2),
                        Color32::from_rgb(cr, cg, cb).gamma_multiply(if k == 0.0 { 1.0 } else { 0.85 }),
                    );
                    ui.painter().rect_stroke(note, CornerRadius::same(2), Stroke::new(0.5, Color32::from_black_alpha(40)), StrokeKind::Inside);
                }
                if resp.clicked() {
                    if on {
                        let open = ui.data(|d| d.get_temp::<bool>(egui::Id::new("jam_sticky_colors"))).unwrap_or(false);
                        ui.data_mut(|d| d.insert_temp(egui::Id::new("jam_sticky_colors"), !open));
                    } else {
                        app.run("tool.set", json!({"tool": "sticky"}));
                    }
                }
                if ui.data(|d| d.get_temp::<bool>(egui::Id::new("jam_sticky_colors"))).unwrap_or(false) {
                    sticky_colors(app, ui.ctx(), pos2(r.min.x, rect.min.y - 8.0), resp.rect);
                }
                x += cell + 4.0;
                continue;
            }
            let active = current == jt.tool && jt.points.is_none_or(|p| p == points) && (jt.tool != ToolId::Rectangle || preset.is_none());
            let r = Rect::from_center_size(pos2(x + cell / 2.0, cy), vec2(cell, cell));
            let tip = if jt.key.is_empty() { tr(jt.label).to_string() } else { format!("{}  {}", tr(jt.label), pretty_shortcut(jt.key)) };
            let resp = tool_cell(ui, &t, r, ("jam_tool", jt.label), jt.icon, active, &tip);
            if active {
                a11y::prefer(&mut app.ui.nav, Region::Toolbar, resp.id);
            }
            if resp.clicked() {
                let params = match jt.points {
                    Some(p) => json!({"tool": jt.tool.id(), "points": p}),
                    None => json!({"tool": jt.tool.id()}),
                };
                app.run("tool.set", params);
            }
            x += cell + 4.0;
        }
    }
}

/// The library of shapes over the toolbar: picking one chooses the tool that draws it.
fn more_shapes(app: &mut FigApp, ctx: &egui::Context, bottom_left: egui::Pos2, button: Rect) {
    let id = egui::Id::new("jam_more_shapes");
    let area = egui::Area::new(egui::Id::new("jam_more_shapes_area"))
        .order(egui::Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(bottom_left)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(Color32::from_rgb(0x2C, 0x2C, 0x2C))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    if let Some(shape) = crate::jam_bar::shape_picker(ui) {
                        let params = match shape {
                            "rectangle" => json!({"tool": "rectangle"}),
                            "ellipse" => json!({"tool": "ellipse"}),
                            "diamond" => json!({"tool": "polygon", "points": 4}),
                            "triangle" => json!({"tool": "polygon", "points": 3}),
                            "star" => json!({"tool": "star"}),
                            other => json!({"tool": "rectangle", "preset": other}),
                        };
                        app.run("tool.set", params);
                        ui.data_mut(|d| d.insert_temp(id, false));
                    }
                })
        });
    let pressed = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
    if let Some(p) = pressed
        && !area.response.rect.contains(p)
        && !button.contains(p)
    {
        ctx.data_mut(|d| d.insert_temp(id, false));
    }
}

/// The sticky notes' colours over the toolbar: the one picked is the next note's.
fn sticky_colors(app: &mut FigApp, ctx: &egui::Context, bottom_left: egui::Pos2, button: Rect) {
    let id = egui::Id::new("jam_sticky_colors");
    let area = egui::Area::new(egui::Id::new("jam_sticky_colors_area"))
        .order(egui::Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(bottom_left)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(Color32::from_rgb(0x2C, 0x2C, 0x2C))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
                        for (i, c) in figcraft_engine::board::STICKY_COLORS.iter().enumerate() {
                            let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                            let resp = resp.on_hover_text(tr(c.name));
                            let [cr, cg, cb, _] = c.color.to_rgba8();
                            ui.painter().rect_filled(r.shrink(2.0), CornerRadius::same(3), Color32::from_rgb(cr, cg, cb));
                            if i == app.session.tool.sticky_color || resp.hovered() {
                                ui.painter().rect_stroke(r, CornerRadius::same(4), Stroke::new(1.5, Color32::WHITE), StrokeKind::Inside);
                            }
                            if resp.clicked() {
                                app.run("tool.set", json!({"tool": "sticky", "color": c.name}));
                                ui.data_mut(|d| d.insert_temp(id, false));
                            }
                        }
                    });
                })
        });
    let pressed = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
    if let Some(p) = pressed
        && !area.response.rect.contains(p)
        && !button.contains(p)
    {
        ctx.data_mut(|d| d.insert_temp(id, false));
    }
}

/// A board's own keys (before the editor's): S a sticky note. True when `key` was one.
pub fn board_key(app: &mut FigApp, key: egui::Key, m: egui::Modifiers) -> bool {
    if !is_board(app) || m.command || m.alt || m.ctrl {
        return false;
    }
    let tool = match (key, m.shift) {
        (egui::Key::S, false) => "sticky",
        _ => return false,
    };
    app.run("tool.set", json!({"tool": tool}));
    true
}
