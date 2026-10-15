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
    let width = count as f32 * (cell + 4.0) + groups.len() as f32 * 12.0 + 16.0;
    let rect = pill(ui, &mut app.ui.nav, canvas, width, &t);
    let current = app.session.tool.tool;
    let points = app.session.tool.polygon_points.unwrap_or(3);
    let mut x = rect.min.x + 8.0;
    let cy = rect.center().y;
    for (gi, g) in groups.iter().enumerate() {
        if gi > 0 {
            divider(ui, &t, x + 2.0, cy);
            x += 12.0;
        }
        for jt in g {
            let active = current == jt.tool && jt.points.is_none_or(|p| p == points);
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
