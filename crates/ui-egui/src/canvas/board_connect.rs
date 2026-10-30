//! Boards: a "+" beside each side of the one selected sticky or shape (#1597). Hovering one shows
//! where the next one goes; clicking it makes the next of the same kind there, joined by a
//! connector, and starts typing in it (`board.addConnected`).

use egui::{Area, Color32, CornerRadius, Id, LayerId, Order, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};
use figcraft_doc::{BoardItem, NodeId};
use figcraft_engine::ToolId;
use figcraft_engine::board::{STICKY, connected_place};
use serde_json::json;

use crate::FigApp;
use crate::i18n::tr;
use crate::theme::Tokens;

/// How far each "+" sits outside the object, from its edge to its centre (screen points): clear
/// of the selection's handles and of the size label under it.
pub(crate) const REACH: f32 = 36.0;
const RADIUS: f32 = 9.0;
const SIDES: [&str; 4] = ["above", "right", "below", "left"];

/// The sticky or shape the handles are for: its id, its box on the page and the next one's size.
fn target(app: &FigApp) -> Option<(NodeId, figcraft_geom::Rect, (f64, f64))> {
    let editable = app.session.active_read_only().is_none();
    if app.session.tool.busy() || !editable || app.session.tool.tool != ToolId::Move || !app.session.doc().is_ok_and(figcraft_engine::board::is_board)
    {
        return None;
    }
    let ds = app.session.ds().ok()?;
    let ([id], None) = (ds.selection.as_slice(), &ds.text_edit) else { return None };
    let n = ds.doc.node(*id)?;
    let next = match n.board {
        Some(BoardItem::Sticky { .. }) => (STICKY, STICKY),
        Some(BoardItem::Shape { .. }) => (n.width, n.height),
        _ => return None,
    };
    Some((*id, ds.doc.world_bounds(*id)?, next))
}

/// Handles show for the selection: the bar above it leaves them room.
pub(crate) fn shown(app: &FigApp) -> bool {
    target(app).is_some()
}

/// The centre of the "+" for `side` of the object's screen box `r`.
fn handle_at(r: Rect, side: &str) -> Pos2 {
    match side {
        "above" => pos2(r.center().x, r.min.y - REACH),
        "right" => pos2(r.max.x + REACH, r.center().y),
        "below" => pos2(r.center().x, r.max.y + REACH),
        _ => pos2(r.min.x - REACH, r.center().y),
    }
}

pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    let Some((id, b, next)) = target(app) else { return };
    let r = Rect::from_two_pos(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1));
    // Too small on screen to tell the handles from the object.
    if r.width() < 2.0 * RADIUS || r.height() < 2.0 * RADIUS {
        return;
    }
    let t = Tokens::get(ctx);
    let accent = crate::theme::canvas_accent(ctx);
    // Not over the toolbar at the bottom of the canvas.
    let toolbar = crate::toolbar::pill_rect(app.canvas.rect, crate::toolbar::bar_width(app)).expand(RADIUS);
    let mut picked = None;
    for side in SIDES {
        let c = handle_at(r, side);
        if !app.canvas.rect.shrink(RADIUS).contains(c) || toolbar.contains(c) {
            continue;
        }
        let area = Area::new(Id::unique(("board_connect", side))).fixed_pos(c - vec2(RADIUS, RADIUS)).order(Order::Middle);
        let clicked = area
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(vec2(2.0 * RADIUS, 2.0 * RADIUS), Sense::click());
                let hot = resp.hovered();
                let p = ui.painter();
                p.circle_filled(rect.center(), RADIUS, if hot { accent } else { t.panel });
                p.circle_stroke(rect.center(), RADIUS, Stroke::new(1.0, if hot { accent } else { t.border }));
                let ink = Stroke::new(1.5, if hot { Color32::WHITE } else { t.text_secondary });
                let arm = RADIUS * 0.45;
                p.line_segment([rect.center() - vec2(arm, 0.0), rect.center() + vec2(arm, 0.0)], ink);
                p.line_segment([rect.center() - vec2(0.0, arm), rect.center() + vec2(0.0, arm)], ink);
                if hot {
                    preview(app, ctx, b, next, side, accent);
                }
                resp.on_hover_text(tr("Add a connected shape")).clicked()
            })
            .inner;
        if clicked {
            picked = Some(side);
        }
    }
    if let Some(side) = picked {
        app.run("board.addConnected", json!({"id": id, "side": side, "edit": true}));
    }
}

/// Where the next one goes for `side`, faintly: its box and the connector's line to it.
fn preview(app: &FigApp, ctx: &egui::Context, b: figcraft_geom::Rect, next: (f64, f64), side: &str, accent: Color32) {
    let Some(((x, y), ..)) = connected_place((b.x0, b.y0), (b.width(), b.height()), next, side) else { return };
    let ghost = Rect::from_two_pos(app.to_screen(x, y), app.to_screen(x + next.0, y + next.1));
    let from = Rect::from_two_pos(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1));
    let (a, z) = match side {
        "above" => (from.center_top(), ghost.center_bottom()),
        "right" => (from.right_center(), ghost.left_center()),
        "below" => (from.center_bottom(), ghost.center_top()),
        _ => (from.left_center(), ghost.right_center()),
    };
    let p = ctx.layer_painter(LayerId::new(Order::Middle, Id::unique("board_connect_preview")));
    let faint = Stroke::new(1.5, accent.gamma_multiply(0.6));
    p.rect_filled(ghost, CornerRadius::same(8), accent.gamma_multiply(0.08));
    let corners = [ghost.left_top(), ghost.right_top(), ghost.right_bottom(), ghost.left_bottom(), ghost.left_top()];
    p.extend(Shape::dashed_line(&corners, faint, 6.0, 4.0));
    p.line_segment([a, z], faint);
}
