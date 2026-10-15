//! A board's context toolbar: a dark bar floating over what's selected, with what can be changed
//! about it without a side panel.
//!
//! - Shapes: the kind of shape (searchable), fill, border (solid, dashed or none, and its colour),
//!   the text's font look and size, bold, strikethrough, link, bulleted list and alignment.
//! - Connectors: colour, thin or thick, solid or dashed, a label, the start and end points and the
//!   line type.
//! - Texts: colour, font look and size, bold, strikethrough, link, list and alignment.
//!
//! Colours come from the board's palette ([`figcraft_engine::board`]), plus a free colour.

use egui::{Align2, Color32, Context, CornerRadius, Id, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_color::{Color, PaintKind};
use figcraft_doc::{ConnectorLineType, Document, Node, NodeId, NodeKind, StrokeCap, TextAlignH, shape_text};
use figcraft_engine::board::{self, FontLook, Swatch, TextSize};
use serde_json::{Value, json};

use crate::FigApp;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme::{ACCENT, regular, semibold};

const BAR: Color32 = Color32::from_rgb(0x2C, 0x2C, 0x2C);
const HOVER: Color32 = Color32::from_rgb(0x45, 0x45, 0x45);
const INK: Color32 = Color32::from_rgb(0xF2, 0xF2, 0xF2);
const DIM: Color32 = Color32::from_rgb(0xA8, 0xA8, 0xA8);
const SEP: Color32 = Color32::from_rgb(0x4A, 0x4A, 0x4A);
const CELL: f32 = 32.0;

/// Which of the bar's popovers is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pop {
    Shape,
    Fill,
    Border,
    Font,
    Size,
    Link,
    StickyColor,
    Start,
    End,
    Line,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    None,
    Shapes,
    Stickies,
    Connectors,
    Texts,
}

/// What the bar works on.
struct Target {
    mode: Mode,
    /// The selected shapes, connectors or texts (shape texts stand for their shapes).
    ids: Vec<NodeId>,
    /// The texts the text controls change: the shapes' texts, the selected texts, or the text
    /// being edited.
    texts: Vec<NodeId>,
    /// Text controls act on the selected characters (text editing).
    editing: bool,
}

fn target(app: &FigApp) -> Option<Target> {
    let ds = app.session.ds().ok()?;
    let doc = &ds.doc;
    let mut ids: Vec<NodeId> = Vec::new();
    for i in &ds.selection {
        let i = shape_text::click_target(doc, *i);
        if !ids.contains(&i) {
            ids.push(i);
        }
    }
    if ids.is_empty() {
        return None;
    }
    let all = |f: &dyn Fn(&Node) -> bool| ids.iter().all(|i| doc.node(*i).is_some_and(f));
    let mode = if all(&|n| matches!(n.kind, NodeKind::Connector { .. })) {
        Mode::Connectors
    } else if all(&|n| shape_text::is_sticky(n)) {
        Mode::Stickies
    } else if all(&|n| {
        shape_text::can_host(&n.kind) && (!matches!(n.kind, NodeKind::Vector { .. }) || figcraft_engine::board_shapes::preset_of(n).is_some())
    }) {
        Mode::Shapes
    } else if all(&|n| matches!(n.kind, NodeKind::Text(_))) {
        Mode::Texts
    } else {
        Mode::None
    };
    let editing = ds.text_edit.as_ref().map(|te| te.id);
    let texts = match (editing, mode) {
        (Some(t), _) => vec![t],
        (None, Mode::Shapes | Mode::Stickies) => ids.iter().filter_map(|i| shape_text::text_of(doc, *i)).collect(),
        (None, Mode::Texts) => ids.clone(),
        _ => Vec::new(),
    };
    Some(Target { mode, ids, texts, editing: editing.is_some() })
}

fn c32(c: Color) -> Color32 {
    let [r, g, b, a] = c.to_rgba8();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// The first solid colour of `paints`.
fn solid(paints: &[figcraft_color::Paint]) -> Option<Color> {
    paints.iter().find_map(|p| match &p.kind {
        PaintKind::Solid { color } if p.visible => Some(*color),
        _ => None,
    })
}

fn text_data(doc: &Document, id: NodeId) -> Option<&figcraft_doc::TextData> {
    match &doc.node(id)?.kind {
        NodeKind::Text(t) => Some(t),
        _ => None,
    }
}

/// A square button of the bar; `paint` draws its face.
fn button(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    w: f32,
    on: bool,
    tip: &str,
    paint: impl FnOnce(&Painter, Rect, Color32),
) -> egui::Response {
    let (r, _) = ui.allocate_exact_size(vec2(w, CELL), Sense::hover());
    let resp = ui.interact(r, ui.id().with(("jam_bar", id)), Sense::click()).on_hover_text(tip);
    crate::a11y::toggle(&resp, tip, on);
    if on {
        ui.painter().rect_filled(r, CornerRadius::same(6), ACCENT);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(6), HOVER);
    }
    paint(ui.painter(), r, INK);
    resp
}

fn sep(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(9.0, CELL), Sense::hover());
    ui.painter().line_segment([pos2(r.center().x, r.min.y + 7.0), pos2(r.center().x, r.max.y - 7.0)], Stroke::new(1.0, SEP));
}

fn swatch_face(p: &Painter, r: Rect, c: Option<Color>, ring: bool) {
    let center = r.center();
    match c {
        Some(c) => {
            p.circle_filled(center, 9.0, c32(c));
            if ring || c.to_rgba8()[..3].iter().all(|v| *v < 40) {
                p.circle_stroke(center, 9.0, Stroke::new(1.0, Color32::from_white_alpha(90)));
            }
        }
        None => {
            p.circle_stroke(center, 9.0, Stroke::new(1.5, DIM));
            p.line_segment([center + vec2(-6.0, 6.0), center + vec2(6.0, -6.0)], Stroke::new(1.5, DIM));
        }
    }
}

fn label_face(text: String, bold: bool) -> impl FnOnce(&Painter, Rect, Color32) {
    move |p, r, ink| {
        p.text(r.center(), Align2::CENTER_CENTER, text, if bold { semibold(12.5) } else { regular(12.0) }, ink);
    }
}

/// The open popover and where it hangs (the left of its button, under the bar).
fn pop_state(ctx: &Context) -> Option<(Pop, f32)> {
    ctx.data(|d| d.get_temp::<(Pop, f32)>(Id::new("jam_bar_pop")))
}

fn set_pop(ctx: &Context, p: Option<(Pop, f32)>) {
    ctx.data_mut(|d| match p {
        Some(v) => {
            d.insert_temp(Id::new("jam_bar_pop"), v);
        }
        None => d.remove::<(Pop, f32)>(Id::new("jam_bar_pop")),
    });
}

fn toggle(ctx: &Context, which: Pop, resp: &egui::Response) {
    let open = pop_state(ctx).map(|(p, _)| p);
    set_pop(ctx, if open == Some(which) { None } else { Some((which, resp.rect.min.x)) });
}

/// Hex text of `c` for `node.set`.
fn hex(c: Color) -> String {
    c.to_css_hex()
}

/// Draw the bar for the selection on a board.
pub fn show(app: &mut FigApp, ctx: &Context) {
    let hide = |ctx: &Context| set_pop(ctx, None);
    if !crate::jam::is_board(app) || app.session.tool.busy() || app.session.ds().is_ok_and(|d| d.vector_edit.is_some()) {
        return hide(ctx);
    }
    let Some(tg) = target(app) else { return hide(ctx) };
    if tg.mode == Mode::None {
        return hide(ctx);
    }
    let Ok(doc) = app.session.doc().map(|d| std::sync::Arc::new(d.clone())) else { return };
    let Some(corners) = figcraft_engine::tools::selection_box(&doc, &tg.ids) else { return hide(ctx) };
    let pts: Vec<Pos2> = corners.iter().map(|c| app.to_screen(c.x, c.y)).collect();
    let (top, bottom) = (pts.iter().map(|p| p.y).fold(f32::MAX, f32::min), pts.iter().map(|p| p.y).fold(f32::MIN, f32::max));
    let cx = pts.iter().map(|p| p.x).sum::<f32>() / pts.len().max(1) as f32;
    let canvas = app.canvas.rect;
    // Above the selection (clear of its connection points), else under it.
    let above = top - 30.0 - CELL - 12.0 > canvas.min.y + 60.0;
    let (pos, pivot) = if above { (pos2(cx, top - 30.0), Align2::CENTER_BOTTOM) } else { (pos2(cx, bottom + 30.0), Align2::CENTER_TOP) };
    let pos = pos2(pos.x.clamp(canvas.min.x + 200.0, (canvas.max.x - 200.0).max(canvas.min.x + 200.0)), pos.y);
    let bar = egui::Area::new(Id::new("jam_bar")).order(egui::Order::Foreground).pivot(pivot).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::NONE.fill(BAR).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
            ui.horizontal(|ui| match tg.mode {
                Mode::Shapes => shape_controls(app, ui, &doc, &tg),
                Mode::Stickies => sticky_controls(app, ui, &doc, &tg),
                Mode::Connectors => connector_controls(app, ui, &doc, &tg),
                Mode::Texts => {
                    let fill = tg.texts.first().and_then(|t| doc.node(*t)).and_then(|n| solid(&n.fills));
                    let resp = button(ui, "fill", CELL, false, tr("Color"), |p, r, _| swatch_face(p, r, fill, true));
                    if resp.clicked() {
                        toggle(ui.ctx(), Pop::Fill, &resp);
                    }
                    sep(ui);
                    text_controls(app, ui, &doc, &tg);
                }
                Mode::None => {}
            });
        });
    });
    let bar_rect = bar.response.rect;
    // The open popover, under the bar.
    let Some((which, x)) = pop_state(ctx) else { return };
    let at = pos2(x, bar_rect.max.y + 6.0);
    let popup = egui::Area::new(Id::new("jam_bar_popover")).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(BAR)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| popover(app, ui, &doc, &tg, which))
    });
    // A press anywhere else closes it.
    let pressed = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
    if let Some(p) = pressed
        && !popup.response.rect.contains(p)
        && !bar_rect.contains(p)
    {
        set_pop(ctx, None);
    }
}

/// A mind map topic's own buttons: a child, a sibling, collapse or show its branch.
fn topic_controls(app: &mut FigApp, ui: &mut Ui, doc: &Document, id: NodeId) {
    let resp = button(ui, "topic_child", CELL, false, tr("Add child topic"), |p, r, ink| {
        let c = r.center();
        p.rect_stroke(Rect::from_center_size(c - vec2(5.0, 0.0), vec2(8.0, 6.0)), CornerRadius::same(2), Stroke::new(1.3, ink), StrokeKind::Inside);
        p.line_segment([c + vec2(-1.0, 0.0), c + vec2(3.0, 0.0)], Stroke::new(1.3, ink));
        p.line_segment([c + vec2(5.0, -3.0), c + vec2(5.0, 3.0)], Stroke::new(1.3, ink));
        p.line_segment([c + vec2(2.0, 0.0), c + vec2(8.0, 0.0)], Stroke::new(1.3, ink));
    });
    if resp.clicked() {
        app.run("mindmap.addChild", json!({"id": id, "edit": true}));
    }
    let resp = button(ui, "topic_sibling", CELL, false, tr("Add sibling topic"), |p, r, ink| {
        let c = r.center();
        p.rect_stroke(Rect::from_center_size(c - vec2(0.0, 4.0), vec2(10.0, 5.0)), CornerRadius::same(2), Stroke::new(1.3, ink), StrokeKind::Inside);
        p.line_segment([c + vec2(0.0, 2.0), c + vec2(0.0, 8.0)], Stroke::new(1.3, ink));
        p.line_segment([c + vec2(-3.0, 5.0), c + vec2(3.0, 5.0)], Stroke::new(1.3, ink));
    });
    if resp.clicked() {
        app.run("mindmap.addSibling", json!({"id": id, "edit": true}));
    }
    let has_kids = !figcraft_engine::mindmap::children(doc, id).is_empty();
    let collapsed = doc.node(id).is_some_and(figcraft_engine::mindmap::is_collapsed);
    let tip = if collapsed { tr("Show branch") } else { tr("Collapse branch") };
    let resp = button(ui, "topic_collapse", CELL, collapsed, tip, |p, r, ink| {
        let c = r.center();
        p.circle_stroke(c, 6.0, Stroke::new(1.3, ink));
        p.line_segment([c - vec2(3.0, 0.0), c + vec2(3.0, 0.0)], Stroke::new(1.3, ink));
        if collapsed {
            p.line_segment([c - vec2(0.0, 3.0), c + vec2(0.0, 3.0)], Stroke::new(1.3, ink));
        }
    });
    if resp.clicked() && has_kids {
        app.run("mindmap.collapse", json!({"id": id}));
    }
    sep(ui);
}

fn shape_controls(app: &mut FigApp, ui: &mut Ui, doc: &Document, tg: &Target) {
    let first = tg.ids.first().and_then(|i| doc.node(*i));
    if let [one] = tg.ids.as_slice()
        && first.is_some_and(figcraft_engine::mindmap::is_topic)
    {
        topic_controls(app, ui, doc, *one);
    }
    let shape_id = match first.map(|n| (&n.kind, figcraft_engine::board_shapes::preset_of(n))) {
        Some((_, Some(pr))) => pr.id,
        Some((NodeKind::Ellipse { .. }, _)) => "ellipse",
        Some((NodeKind::Polygon { count: 4 }, _)) => "diamond",
        Some((NodeKind::Polygon { .. }, _)) => "triangle",
        Some((NodeKind::Star { .. }, _)) => "star",
        _ => "rectangle",
    };
    let resp = button(ui, "shape", CELL + 12.0, false, tr("Shape"), |p, r, ink| {
        shape_preview(p, Rect::from_center_size(r.center() - vec2(5.0, 0.0), vec2(16.0, 14.0)), shape_id, ink);
        icons::paint(p, Rect::from_center_size(pos2(r.max.x - 9.0, r.center().y), vec2(9.0, 9.0)), Icon::Chevron, DIM);
    });
    if resp.clicked() {
        toggle(ui.ctx(), Pop::Shape, &resp);
    }
    sep(ui);
    let fill = first.and_then(|n| solid(&n.fills));
    let resp = button(ui, "fill", CELL, false, tr("Fill"), |p, r, _| swatch_face(p, r, fill, false));
    if resp.clicked() {
        toggle(ui.ctx(), Pop::Fill, &resp);
    }
    let border = first.and_then(|n| solid(&n.strokes));
    let dashed = first.is_some_and(|n| !n.stroke.dashes.is_empty());
    let resp = button(ui, "border", CELL, false, tr("Border"), |p, r, ink| {
        let rr = Rect::from_center_size(r.center(), vec2(16.0, 16.0));
        match border {
            None => {
                p.rect_stroke(rr, CornerRadius::same(3), Stroke::new(1.0, DIM), StrokeKind::Inside);
            }
            Some(c) => {
                if dashed {
                    for k in 0..4 {
                        let t = k as f32 * 4.0;
                        p.line_segment([rr.left_top() + vec2(t, 0.0), rr.left_top() + vec2(t + 2.5, 0.0)], Stroke::new(2.0, c32(c)));
                        p.line_segment([rr.left_bottom() + vec2(t, 0.0), rr.left_bottom() + vec2(t + 2.5, 0.0)], Stroke::new(2.0, c32(c)));
                        p.line_segment([rr.left_top() + vec2(0.0, t), rr.left_top() + vec2(0.0, t + 2.5)], Stroke::new(2.0, c32(c)));
                        p.line_segment([rr.right_top() + vec2(0.0, t), rr.right_top() + vec2(0.0, t + 2.5)], Stroke::new(2.0, c32(c)));
                    }
                } else {
                    p.rect_stroke(rr, CornerRadius::same(3), Stroke::new(2.0, c32(c)), StrokeKind::Inside);
                }
                let _ = ink;
            }
        }
    });
    if resp.clicked() {
        toggle(ui.ctx(), Pop::Border, &resp);
    }
    sep(ui);
    text_controls(app, ui, doc, tg);
}

/// Font look, size, bold, strikethrough, link, list and alignment (for the texts in `tg`).
fn text_controls(app: &mut FigApp, ui: &mut Ui, doc: &Document, tg: &Target) {
    let t = tg.texts.first().and_then(|t| text_data(doc, *t));
    let resp = button(ui, "font", CELL, false, tr("Font"), label_face("Aa".into(), true));
    if resp.clicked() {
        toggle(ui.ctx(), Pop::Font, &resp);
    }
    let size = t.map(|t| t.style.font_size).unwrap_or(TextSize::Small.px());
    let size_label = TextSize::of(size).map(|s| tr(s.label()).to_string()).unwrap_or_else(|| format!("{size:.0}"));
    let w = ui.painter().layout_no_wrap(size_label.clone(), regular(12.0), INK).size().x + 30.0;
    let resp = button(ui, "size", w.max(CELL), false, tr("Font size"), |p, r, ink| {
        p.text(r.center() - vec2(5.0, 0.0), Align2::CENTER_CENTER, size_label, regular(12.0), ink);
        icons::paint(p, Rect::from_center_size(pos2(r.max.x - 9.0, r.center().y), vec2(9.0, 9.0)), Icon::Chevron, DIM);
    });
    if resp.clicked() {
        toggle(ui.ctx(), Pop::Size, &resp);
    }
    sep(ui);
    let bold = t.is_some_and(|t| t.style.font_style.to_ascii_lowercase().contains("bold"));
    let strike = t.is_some_and(|t| t.style.decoration == figcraft_doc::TextDecoration::Strikethrough);
    let ids = |tg: &Target| if tg.editing { json!({}) } else { json!({"ids": tg.texts}) };
    let has_text = !tg.texts.is_empty();
    if button(ui, "bold", CELL, bold, tr("Bold"), label_face("B".into(), true)).clicked() && has_text {
        app.run("text.bold", ids(tg));
    }
    let resp = button(ui, "strike", CELL, strike, tr("Strikethrough"), |p, r, ink| {
        p.text(r.center(), Align2::CENTER_CENTER, "S", regular(13.0), ink);
        p.line_segment([r.center() + vec2(-6.0, 0.5), r.center() + vec2(6.0, 0.5)], Stroke::new(1.3, ink));
    });
    if resp.clicked() && has_text {
        app.run("text.strikethrough", ids(tg));
    }
    let resp = button(ui, "link", CELL, false, tr("Link"), |p, r, ink| {
        let c = r.center();
        for (dx, dy) in [(-3.0, 3.0), (3.0, -3.0)] {
            let rr = Rect::from_center_size(c + vec2(dx, dy), vec2(9.0, 5.0));
            let rot = egui::emath::Rot2::from_angle(-std::f32::consts::FRAC_PI_4);
            let pts: Vec<Pos2> = [rr.left_top(), rr.right_top(), rr.right_bottom(), rr.left_bottom()]
                .iter()
                .map(|q| c + vec2(dx, dy) + rot * (*q - (c + vec2(dx, dy))))
                .collect();
            p.add(egui::Shape::closed_line(pts, Stroke::new(1.3, ink)));
        }
    });
    if resp.clicked() && has_text {
        toggle(ui.ctx(), Pop::Link, &resp);
    }
    let resp = button(ui, "list", CELL, false, tr("Bulleted list"), |p, r, ink| {
        for k in 0..3 {
            let y = r.center().y - 5.0 + k as f32 * 5.0;
            p.circle_filled(pos2(r.center().x - 6.0, y), 1.4, ink);
            p.line_segment([pos2(r.center().x - 2.5, y), pos2(r.center().x + 7.0, y)], Stroke::new(1.3, ink));
        }
    });
    if resp.clicked() && has_text {
        app.run("text.bulletList", ids(tg));
    }
    sep(ui);
    let align = t.map(|t| t.align_h).unwrap_or(TextAlignH::Center);
    for (a, icon, cmd, tip) in [
        (TextAlignH::Left, Icon::TextAlignLeft, "text.alignLeft", tr("Align left")),
        (TextAlignH::Center, Icon::TextAlignCenter, "text.alignCenter", tr("Align center")),
        (TextAlignH::Right, Icon::TextAlignRight, "text.alignRight", tr("Align right")),
    ] {
        let resp = button(ui, cmd, CELL, align == a && has_text, tip, |p, r, ink| {
            icons::paint(p, Rect::from_center_size(r.center(), vec2(16.0, 16.0)), icon, ink)
        });
        if resp.clicked() && has_text {
            app.run(cmd, json!({"ids": tg.texts}));
        }
    }
}

/// A sticky note's colour, square or wide, and whether it's signed; then the text controls.
fn sticky_controls(app: &mut FigApp, ui: &mut Ui, doc: &Document, tg: &Target) {
    let first = tg.ids.first().and_then(|i| doc.node(*i));
    let fill = first.and_then(|n| solid(&n.fills));
    let resp = button(ui, "sticky_color", CELL, false, tr("Color"), |p, r, _| {
        if let Some(c) = fill {
            p.rect_filled(Rect::from_center_size(r.center(), vec2(18.0, 18.0)), CornerRadius::same(3), c32(c));
        }
    });
    if resp.clicked() {
        toggle(ui.ctx(), Pop::StickyColor, &resp);
    }
    sep(ui);
    let wide = first.is_some_and(|n| n.width > board::STICKY_SIZE + 1.0);
    for (w, tip) in [(false, tr("Square")), (true, tr("Wide"))] {
        let resp = button(ui, ("sticky_wide", w), CELL, wide == w, tip, |p, r, ink| {
            let sz = if w { vec2(20.0, 11.0) } else { vec2(13.0, 13.0) };
            p.rect_stroke(Rect::from_center_size(r.center(), sz), CornerRadius::same(2), Stroke::new(1.4, ink), StrokeKind::Inside);
        });
        if resp.clicked() && wide != w {
            app.run("sticky.set", json!({"ids": tg.ids, "wide": w}));
        }
    }
    let signed = first.is_some_and(|n| shape_text::sticky_author(n).is_some());
    let resp = button(ui, "sticky_author", CELL, signed, tr("Show author"), |p, r, ink| {
        let c = r.center();
        p.circle_stroke(c + vec2(0.0, -3.5), 3.2, Stroke::new(1.4, ink));
        p.add(egui::Shape::line(
            (0..=12)
                .map(|k| {
                    let a = std::f32::consts::PI * (1.0 + k as f32 / 12.0);
                    c + vec2(6.5 * a.cos(), 7.5 + 5.0 * a.sin())
                })
                .collect(),
            Stroke::new(1.4, ink),
        ));
    });
    if resp.clicked() {
        app.run("sticky.set", json!({"ids": tg.ids, "author": !signed}));
    }
    sep(ui);
    text_controls(app, ui, doc, tg);
}

fn cap_name(c: StrokeCap) -> Value {
    serde_json::to_value(c).unwrap_or(Value::Null)
}

fn connector_controls(app: &mut FigApp, ui: &mut Ui, doc: &Document, tg: &Target) {
    let first = tg.ids.first().and_then(|i| doc.node(*i));
    let color = first.and_then(|n| solid(&n.strokes));
    let resp = button(ui, "color", CELL, false, tr("Color"), |p, r, _| swatch_face(p, r, color, false));
    if resp.clicked() {
        toggle(ui.ctx(), Pop::Fill, &resp);
    }
    sep(ui);
    let weight = first.map(|n| n.stroke.weight).unwrap_or(board_thin());
    let thick = weight >= figcraft_doc::connector::THICK - 1e-6;
    let resp = button(ui, "weight", CELL, false, if thick { tr("Thick") } else { tr("Thin") }, |p, r, ink| {
        p.line_segment([r.center() + vec2(-8.0, 0.0), r.center() + vec2(8.0, 0.0)], Stroke::new(if thick { 4.0 } else { 1.5 }, ink));
    });
    if resp.clicked() {
        let w = if thick { figcraft_doc::connector::THIN } else { figcraft_doc::connector::THICK };
        app.set_props(&tg.ids, json!({"strokeWeight": w}));
    }
    let dashed = first.is_some_and(|n| !n.stroke.dashes.is_empty());
    let resp = button(ui, "dash", CELL, dashed, if dashed { tr("Dashed") } else { tr("Solid") }, |p, r, ink| {
        if dashed {
            for k in 0..3 {
                let x = r.center().x - 8.0 + k as f32 * 6.0;
                p.line_segment([pos2(x, r.center().y), pos2(x + 3.5, r.center().y)], Stroke::new(1.8, ink));
            }
        } else {
            p.line_segment([r.center() + vec2(-8.0, 0.0), r.center() + vec2(8.0, 0.0)], Stroke::new(1.8, ink));
        }
    });
    if resp.clicked() {
        app.set_props(&tg.ids, json!({"dashes": if dashed { vec![] } else { vec![8.0, 6.0] }}));
    }
    sep(ui);
    // A label on the line (edited in place, made if there's none).
    let labelled = tg.ids.first().is_some_and(|i| shape_text::text_of(doc, *i).is_some());
    let resp = button(ui, "label", CELL, labelled, tr("Label"), label_face("T".into(), true));
    if resp.clicked()
        && let [one] = tg.ids.as_slice()
    {
        app.run("shape.editText", json!({"id": one}));
    }
    sep(ui);
    let line = match first.map(|n| &n.kind) {
        Some(NodeKind::Connector { line_type, .. }) => *line_type,
        _ => ConnectorLineType::Elbowed,
    };
    for (which, icon, tip) in
        [(Pop::Start, Icon::ArrowLeft, tr("Start point")), (Pop::Line, Icon::Connector, tr("Line type")), (Pop::End, Icon::Arrow, tr("End point"))]
    {
        let label = match which {
            Pop::Line => tr(line.label()).to_string(),
            _ => String::new(),
        };
        let resp = button(ui, ("pop", which as u8), CELL + 12.0, false, tip, |p, r, ink| {
            icons::paint(p, Rect::from_center_size(r.center() - vec2(5.0, 0.0), vec2(16.0, 16.0)), icon, ink);
            icons::paint(p, Rect::from_center_size(pos2(r.max.x - 9.0, r.center().y), vec2(9.0, 9.0)), Icon::Chevron, DIM);
        });
        let resp = if label.is_empty() { resp } else { resp.on_hover_text(label) };
        if resp.clicked() {
            toggle(ui.ctx(), which, &resp);
        }
    }
}

fn board_thin() -> f64 {
    figcraft_doc::connector::THIN
}

/// A row of palette swatches; returns the colour clicked.
fn swatch_row(ui: &mut Ui, row: &[Swatch], current: Option<Color>) -> Option<Color> {
    let mut out = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
        for s in row {
            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
            let resp = resp.on_hover_text(tr(s.name));
            ui.painter().circle_filled(r.center(), 9.0, c32(s.color));
            if s.color.to_rgba8()[..3].iter().all(|v| *v > 230) || s.color.to_rgba8()[..3].iter().all(|v| *v < 40) {
                ui.painter().circle_stroke(r.center(), 9.0, Stroke::new(1.0, Color32::from_white_alpha(80)));
            }
            if current.is_some_and(|c| c.to_rgba8()[..3] == s.color.to_rgba8()[..3]) || resp.hovered() {
                ui.painter().circle_stroke(r.center(), 11.0, Stroke::new(1.5, INK));
            }
            if resp.clicked() {
                out = Some(s.color);
            }
        }
    });
    out
}

/// The palette: strong colours, then light ones and a free colour (hex). Returns the colour chosen.
fn palette(ui: &mut Ui, current: Option<Color>, none: bool) -> Option<Option<Color>> {
    let mut out = None;
    if let Some(c) = swatch_row(ui, &board::STRONG, current) {
        out = Some(Some(c));
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if let Some(c) = swatch_row(ui, &board::LIGHT, current) {
            out = Some(Some(c));
        }
        // The free colour: a hex field.
        let id = Id::new("jam_free_color");
        let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| current.map(|c| c.to_css_hex()).unwrap_or_default());
        let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(64.0).hint_text("#hex").font(regular(11.5)));
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
        if edit.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
            && let Some(c) = Color::parse_css(text.trim())
        {
            out = Some(Some(c));
        }
    });
    if none {
        ui.add_space(4.0);
        if ui.add(egui::Button::new(egui::RichText::new(tr("No fill")).color(INK).size(11.5)).fill(HOVER)).clicked() {
            out = Some(None);
        }
    }
    out
}

fn popover(app: &mut FigApp, ui: &mut Ui, doc: &Document, tg: &Target, which: Pop) {
    ui.style_mut().visuals.override_text_color = Some(INK);
    // Fields on the dark popover: dark too.
    ui.style_mut().visuals.extreme_bg_color = HOVER;
    ui.style_mut().visuals.text_edit_bg_color = Some(HOVER);
    ui.style_mut().visuals.selection.stroke = Stroke::new(1.0, INK);
    let first = tg.ids.first().and_then(|i| doc.node(*i));
    let done = |ui: &Ui| set_pop(ui.ctx(), None);
    match which {
        Pop::Shape => {
            if let Some(name) = shape_picker(ui) {
                app.run("shape.setType", json!({"ids": tg.ids, "type": name}));
                done(ui);
            }
        }
        Pop::Fill => {
            let (current, key, ids, none) = match tg.mode {
                Mode::Connectors => (first.and_then(|n| solid(&n.strokes)), "stroke", tg.ids.clone(), false),
                Mode::Texts => (tg.texts.first().and_then(|t| doc.node(*t)).and_then(|n| solid(&n.fills)), "fill", tg.texts.clone(), false),
                _ => (first.and_then(|n| solid(&n.fills)), "fill", tg.ids.clone(), true),
            };
            if let Some(c) = palette(ui, current, none) {
                match c {
                    Some(c) => app.set_props(&ids, json!({ key: hex(c) })),
                    None => app.set_props(&ids, json!({"fills": []})),
                }
                done(ui);
            }
        }
        Pop::Border => {
            let border = first.and_then(|n| solid(&n.strokes));
            let dashed = first.is_some_and(|n| !n.stroke.dashes.is_empty());
            let color = border.or_else(|| first.and_then(|n| solid(&n.fills)).map(board::darker)).unwrap_or(board::STRONG[0].color);
            ui.horizontal(|ui| {
                for (label, on, props) in [
                    (
                        tr("Solid"),
                        border.is_some() && !dashed,
                        json!({"stroke": hex(color), "strokeWeight": 2, "strokeAlign": "inside", "dashes": []}),
                    ),
                    (
                        tr("Dashed"),
                        border.is_some() && dashed,
                        json!({"stroke": hex(color), "strokeWeight": 2, "strokeAlign": "inside", "dashes": [6, 4]}),
                    ),
                    (tr("None"), border.is_none(), json!({"strokes": []})),
                ] {
                    let w = ui.painter().layout_no_wrap(label.to_string(), regular(12.0), INK).size().x + 20.0;
                    if button(ui, ("border_style", label), w, on, label, label_face(label.to_string(), false)).clicked() {
                        app.set_props(&tg.ids, props);
                    }
                }
            });
            ui.add_space(6.0);
            if let Some(Some(c)) = palette(ui, border, false) {
                let mut props = json!({"stroke": hex(c)});
                if border.is_none()
                    && let Some(o) = props.as_object_mut()
                {
                    o.insert("strokeWeight".into(), json!(2));
                    o.insert("strokeAlign".into(), json!("inside"));
                }
                app.set_props(&tg.ids, props);
                done(ui);
            }
        }
        Pop::Font => {
            let fam = tg.texts.first().and_then(|t| text_data(doc, *t)).map(|t| t.style.font_family.clone());
            let current = fam.as_deref().and_then(FontLook::of);
            for look in FontLook::ALL {
                let family = look.family();
                let resp = button(ui, ("look", look.label()), 160.0, current == Some(look), tr(look.label()), |p, r, ink| {
                    p.text(pos2(r.min.x + 10.0, r.center().y), Align2::LEFT_CENTER, tr(look.label()), regular(12.5), ink);
                });
                if resp.clicked() && !tg.texts.is_empty() {
                    app.set_props(&tg.texts, json!({"fontFamily": family}));
                    done(ui);
                }
            }
        }
        Pop::Size => {
            let size = tg.texts.first().and_then(|t| text_data(doc, *t)).map(|t| t.style.font_size);
            for s in TextSize::ALL {
                let resp = button(ui, ("size", s.label()), 160.0, size == Some(s.px()), tr(s.label()), |p, r, ink| {
                    p.text(pos2(r.min.x + 10.0, r.center().y), Align2::LEFT_CENTER, tr(s.label()), regular(12.5), ink);
                    p.text(pos2(r.max.x - 10.0, r.center().y), Align2::RIGHT_CENTER, format!("{:.0}", s.px()), regular(11.5), DIM);
                });
                if resp.clicked() && !tg.texts.is_empty() {
                    app.set_props(&tg.texts, json!({"fontSize": s.px()}));
                    done(ui);
                }
            }
            // Any size.
            let id = Id::new("jam_size_field");
            let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| size.map(|s| format!("{s:.0}")).unwrap_or_default());
            let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(150.0).font(regular(12.0)));
            ui.data_mut(|d| d.insert_temp(id, text.clone()));
            if edit.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Ok(v) = text.trim().parse::<f64>()
                && v.is_finite()
                && !tg.texts.is_empty()
            {
                app.set_props(&tg.texts, json!({"fontSize": v.clamp(1.0, 1000.0)}));
                ui.data_mut(|d| d.remove::<String>(id));
                done(ui);
            }
        }
        Pop::Link => {
            let id = Id::new("jam_link_field");
            let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
            let edit = ui.add(egui::TextEdit::singleline(&mut text).hint_text("https://").desired_width(220.0).font(regular(12.0)));
            edit.request_focus();
            ui.data_mut(|d| d.insert_temp(id, text.clone()));
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.trim().is_empty() {
                let mut p = json!({"url": text.trim()});
                if !tg.editing
                    && let Some(o) = p.as_object_mut()
                {
                    o.insert("ids".into(), json!(tg.texts));
                }
                app.run("text.link", p);
                ui.data_mut(|d| d.remove::<String>(id));
                done(ui);
            }
        }
        Pop::StickyColor => {
            let current = first.and_then(|n| solid(&n.fills));
            if let Some(c) = swatch_row(ui, &board::STICKY_COLORS, current) {
                app.run("sticky.set", json!({"ids": tg.ids, "color": hex(c)}));
                done(ui);
            }
        }
        Pop::Start | Pop::End => {
            let key = if which == Pop::Start { "startCap" } else { "endCap" };
            let current = first.map(|n| if which == Pop::Start { n.stroke.cap_start } else { n.stroke.cap_end });
            for cap in [
                StrokeCap::None,
                StrokeCap::LineArrow,
                StrokeCap::TriangleArrow,
                StrokeCap::ReversedTriangle,
                StrokeCap::CircleFilled,
                StrokeCap::DiamondFilled,
            ] {
                let resp = button(ui, (key, cap.label()), 170.0, current == Some(cap), tr(cap.label()), |p, r, ink| {
                    p.text(pos2(r.min.x + 10.0, r.center().y), Align2::LEFT_CENTER, tr(cap.label()), regular(12.5), ink);
                });
                if resp.clicked() {
                    app.set_props(&tg.ids, json!({ key: cap_name(cap) }));
                    done(ui);
                }
            }
        }
        Pop::Line => {
            let current = match first.map(|n| &n.kind) {
                Some(NodeKind::Connector { line_type, .. }) => Some(*line_type),
                _ => None,
            };
            for l in ConnectorLineType::ALL {
                let resp = button(ui, ("line", l.label()), 140.0, current == Some(l), tr(l.label()), |p, r, ink| {
                    p.text(pos2(r.min.x + 10.0, r.center().y), Align2::LEFT_CENTER, tr(l.label()), regular(12.5), ink);
                });
                if resp.clicked() {
                    app.set_props(&tg.ids, json!({"connectorLineType": format!("{l:?}").to_ascii_lowercase()}));
                    done(ui);
                }
            }
        }
    }
}

/// The shapes a board offers: the editor's own kinds, then the library shapes. (id, name, group)
pub fn shape_list() -> Vec<(&'static str, &'static str, figcraft_engine::board_shapes::Group)> {
    use figcraft_engine::board_shapes::{Group, PRESETS};
    let basic = [("rectangle", "Square"), ("ellipse", "Ellipse"), ("diamond", "Diamond"), ("triangle", "Triangle"), ("star", "Star")];
    basic.into_iter().map(|(i, l)| (i, l, Group::Basic)).chain(PRESETS.iter().map(|p| (p.id, p.label, p.group))).collect()
}

/// A shape's little picture in `r`.
pub fn shape_preview(p: &Painter, r: Rect, id: &str, ink: Color32) {
    let icon = match id {
        "rectangle" => Some(Icon::Rectangle),
        "ellipse" => Some(Icon::Ellipse),
        "diamond" => Some(Icon::Diamond),
        "triangle" => Some(Icon::Polygon),
        "star" => Some(Icon::Star),
        _ => None,
    };
    if let Some(icon) = icon {
        return icons::paint(p, r, icon, ink);
    }
    let Some(path) = figcraft_engine::board_shapes::path(id, f64::from(r.width()), f64::from(r.height())) else { return };
    let mut lines: Vec<Vec<Pos2>> = Vec::new();
    figcraft_geom::kurbo::flatten(path.iter(), 0.25, |el| {
        use figcraft_geom::kurbo::PathEl::*;
        let at = |q: figcraft_geom::Point| pos2(r.min.x + q.x as f32, r.min.y + q.y as f32);
        match el {
            MoveTo(q) => lines.push(vec![at(q)]),
            LineTo(q) => {
                if let Some(l) = lines.last_mut() {
                    l.push(at(q));
                }
            }
            ClosePath => {
                if let Some(l) = lines.last_mut()
                    && let Some(f) = l.first().copied()
                {
                    l.push(f);
                }
            }
            _ => {}
        }
    });
    for l in lines {
        p.add(egui::Shape::line(l, Stroke::new(1.2, ink)));
    }
}

/// The searchable grid of shapes; returns the one picked.
pub fn shape_picker(ui: &mut Ui) -> Option<&'static str> {
    let id = Id::new("jam_shape_search");
    let mut q = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
    ui.style_mut().visuals.extreme_bg_color = HOVER;
    ui.style_mut().visuals.override_text_color = Some(INK);
    ui.add(egui::TextEdit::singleline(&mut q).hint_text(tr("Search for a shape")).desired_width(236.0));
    ui.data_mut(|d| d.insert_temp(id, q.clone()));
    let q = q.trim().to_lowercase();
    let list: Vec<_> =
        shape_list().into_iter().filter(|(i, l, _)| q.is_empty() || i.to_lowercase().contains(&q) || tr(l).to_lowercase().contains(&q)).collect();
    let mut picked = None;
    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
        for g in [figcraft_engine::board_shapes::Group::Basic, figcraft_engine::board_shapes::Group::Flowchart] {
            let items: Vec<_> = list.iter().filter(|(_, _, gg)| *gg == g).collect();
            if items.is_empty() {
                continue;
            }
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tr(g.label())).color(DIM).size(11.0));
            for row in items.chunks(6) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    for (sid, label, _) in row {
                        let resp = button(ui, ("pick", *sid), 36.0, false, tr(label), |p, r, ink| {
                            shape_preview(p, Rect::from_center_size(r.center(), vec2(20.0, 16.0)), sid, ink)
                        });
                        if resp.clicked() {
                            picked = Some(*sid);
                        }
                    }
                });
            }
        }
    });
    if picked.is_some() {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    picked
}
