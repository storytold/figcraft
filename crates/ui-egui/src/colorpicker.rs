//! The colour picker popover: paint type, saturation/brightness square, hue and alpha sliders,
//! hex and RGB fields, gradient stops, image and pattern fills and document colours.

use egui::{Align2, Color32, CornerRadius, Id, Mesh, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_color::{
    Color, ColorStop, Gradient, GradientKind, Hsb, ImageFilters, ImageScaleMode, Paint, PaintKind, PatternAlign, PatternPaint, PatternTile,
};
use figcraft_doc::{NodeId, NodeKind};
use serde_json::json;

use crate::FigApp;
use crate::a11y::Tip;
use crate::i18n::{tr, tr_noop, trf};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, doc_c32, regular, semibold};
use crate::widgets;

#[derive(Clone, Debug, PartialEq)]
pub enum PickerTarget {
    PageBackground,
    Paint {
        fills: bool,
        index: usize,
        ids: Vec<NodeId>,
    },
    Effect {
        ids: Vec<NodeId>,
        index: usize,
        /// A duo noise's second colour.
        secondary: bool,
    },
    /// The fill of a character range of a text layer being edited.
    TextRange {
        id: NodeId,
        start: usize,
        end: usize,
    },
    /// A colour variable's value in one mode.
    Variable {
        id: String,
        mode: String,
    },
    /// One paint of a colour style (edited from the style picker).
    Style {
        id: String,
        index: usize,
    },
    /// A colour field of a plugin's panel or parameters form. A drag only shows `pending`; the
    /// plugin hears of the colour when the drag ends.
    PluginField {
        plugin: String,
        control: String,
        pending: Option<String>,
    },
}

/// The paints of colour style `id`.
fn style_paints(doc: &figcraft_doc::Document, id: &str) -> Option<Vec<Paint>> {
    match &doc.style(id)?.kind {
        figcraft_doc::StyleKind::Paint { paints } => Some(paints.clone()),
        _ => None,
    }
}

/// The fills a text range paints with (its own, else the layer's).
pub fn range_fills(doc: &figcraft_doc::Document, id: NodeId, start: usize) -> Option<Vec<Paint>> {
    let n = doc.node(id)?;
    let t = n.kind.text()?;
    Some(t.style_at(start).fills.clone().unwrap_or_else(|| n.fills.clone()))
}

fn current_paint(app: &FigApp, target: &PickerTarget) -> Option<Paint> {
    let doc = app.session.doc().ok()?;
    match target {
        PickerTarget::PageBackground => match &doc.node(app.session.page().ok()?)?.kind {
            NodeKind::Page { background, .. } => Some(Paint::solid(*background)),
            _ => None,
        },
        PickerTarget::Paint { fills, index, ids } => {
            let n = doc.node(*ids.first()?)?;
            let list = if *fills { &n.fills } else { &n.strokes };
            list.get(*index).cloned()
        }
        PickerTarget::Effect { ids, index, secondary } => doc.node(*ids.first()?)?.effects.get(*index)?.color(*secondary).map(Paint::solid),
        PickerTarget::TextRange { id, start, .. } => range_fills(doc, *id, *start)?.into_iter().next(),
        PickerTarget::Variable { id, mode } => match doc.variables.get(id)?.values.get(mode)? {
            figcraft_doc::VariableValue::Color(c) => {
                let mut p = Paint::solid(c.with_alpha(1.0));
                p.opacity = c.a;
                Some(p)
            }
            _ => None,
        },
        PickerTarget::Style { id, index } => style_paints(doc, id)?.get(*index).cloned(),
        PickerTarget::PluginField { plugin, control, pending } => {
            let hex = match pending {
                Some(hex) => hex.clone(),
                None => crate::plugins_ui::field_value(app, plugin, control)?,
            };
            let c = Color::from_hex(&hex)?;
            let mut p = Paint::solid(c.with_alpha(1.0));
            p.opacity = c.a;
            Some(p)
        }
    }
}

/// Write `paint` to the target. `live` edits are part of an ongoing drag.
fn write(app: &mut FigApp, target: &PickerTarget, paint: &Paint, live: bool) {
    if let PickerTarget::PluginField { plugin, control, .. } = target {
        let c = paint.solid_color().unwrap_or(Color::BLACK);
        let hex = c.with_alpha(c.a * paint.opacity).to_css_hex();
        let (plugin, control) = (plugin.clone(), control.clone());
        if !live {
            crate::plugins_ui::set_field(app, &plugin, &control, &hex);
        }
        app.ui.picker = Some(PickerTarget::PluginField { plugin, control, pending: live.then_some(hex) });
        return;
    }
    if live {
        let _ = app.session.begin_interaction("Color");
    }
    let r = match target {
        PickerTarget::PageBackground => {
            let c = paint.solid_color().unwrap_or(Color::DEFAULT_PAGE);
            app.session.execute("page.setBackground", &json!({"color": c.to_css_hex()}))
        }
        PickerTarget::Paint { fills, index, ids } => {
            app.session.execute(if *fills { "fill.set" } else { "stroke.set" }, &json!({"ids": ids, "index": index, "paint": paint}))
        }
        PickerTarget::Effect { ids, index, secondary } => {
            let Some(mut e) = ids.first().and_then(|i| app.session.doc().ok()?.node(*i)?.effects.get(*index).cloned()) else { return };
            if let Some(c) = e.color_mut(*secondary) {
                *c = paint.solid_color().unwrap_or(Color::BLACK);
            }
            app.session.execute("effect.set", &json!({"ids": ids, "index": index, "effect": e}))
        }
        PickerTarget::TextRange { id, start, end } => {
            app.session.execute("text.style", &json!({"id": id, "start": start, "end": end, "textFills": [paint]}))
        }
        PickerTarget::Variable { id, mode } => {
            let c = paint.solid_color().unwrap_or(Color::BLACK);
            app.session.execute("variable.set", &json!({"variable": id, "mode": mode, "value": c.with_alpha(c.a * paint.opacity).to_css_hex()}))
        }
        PickerTarget::PluginField { .. } => return,
        PickerTarget::Style { id, index } => {
            let Some(mut paints) = app.session.doc().ok().and_then(|d| style_paints(d, id)) else { return };
            if let Some(slot) = paints.get_mut(*index) {
                *slot = paint.clone();
            }
            app.session.execute("style.update", &json!({"style": id, "value": paints}))
        }
    };
    if let Err(e) = r {
        app.toast(e.to_string());
    }
    // Drag previews (`live`) are not choices: a drag records only the colour it ends on. Not "an
    // interaction is open": editing text holds one, and its colour clicks are choices.
    if !live && let Some(c) = paint.solid_color() {
        remember_recent(app, c);
    }
}

fn remember_recent(app: &mut FigApp, c: Color) {
    // By 8-bit value: a document stores colours rounded to 8 bits, so the colour picked and the one read
    // back differ by a fraction and would show as two identical swatches.
    app.session.recent_colors.retain(|x| x.to_rgba8() != c.to_rgba8());
    app.session.recent_colors.insert(0, c);
    app.session.recent_colors.truncate(16);
}

fn finish(app: &mut FigApp) {
    if let Some(PickerTarget::PluginField { plugin, control, pending: Some(hex) }) = app.ui.picker.clone() {
        crate::plugins_ui::set_field(app, &plugin, &control, &hex);
        app.ui.picker = Some(PickerTarget::PluginField { plugin, control, pending: None });
    }
    if app.session.ds().map(|d| d.interaction.is_some()).unwrap_or(false) {
        if let Some(t) = app.ui.picker.clone()
            && !matches!(t, PickerTarget::PluginField { .. })
            && let Some(c) = current_paint(app, &t).and_then(|p| p.solid_color())
        {
            remember_recent(app, c);
        }
        let _ = app.session.commit_interaction();
    }
}

/// The selected gradient stop in the picker (shared with the on-canvas stops).
pub fn stop_id() -> Id {
    Id::new("cp_stop")
}

/// Apply a swatch colour: a solid paint's colour, or the selected gradient stop's.
fn set_color(app: &mut FigApp, target: &PickerTarget, paint: &mut Paint, c: Color, stop: Id, ui: &Ui) {
    match &mut paint.kind {
        PaintKind::Solid { color } => {
            *color = c.with_alpha(1.0);
            paint.opacity = c.a;
        }
        PaintKind::Gradient(g) => {
            let sel: usize = ui.data(|d| d.get_temp(stop)).unwrap_or(0);
            if let Some(s) = g.stops.get_mut(sel) {
                s.color = c;
            }
        }
        PaintKind::Image(_) | PaintKind::Pattern(_) => return,
    }
    write(app, target, paint, false);
}

/// The page's most used solid colours, recomputed only when the document snapshot changes.
fn page_colors(app: &FigApp, ui: &Ui) -> Vec<Color> {
    let Ok(ds) = app.session.ds() else { return vec![] };
    let key = (std::sync::Arc::as_ptr(&ds.doc) as usize, ds.page);
    let cache_id = Id::new("cp_doc_colors");
    if let Some((k, colors)) = ui.data(|d| d.get_temp::<((usize, figcraft_doc::NodeId), Vec<Color>)>(cache_id))
        && k == key
    {
        return colors;
    }
    let colors: Vec<Color> = figcraft_engine::cmd::styles::document_colors(&ds.doc, ds.page).into_iter().take(21).map(|(c, _)| c).collect();
    ui.data_mut(|d| d.insert_temp(cache_id, (key, colors.clone())));
    colors
}

/// Colour models the value row can show.
const MODELS: [&str; 5] = ["Hex", "RGB", "HSL", "HSB", "CSS"];

/// The model dropdown with its fields and the alpha field. Returns a new colour and whether the
/// edit is done (false while a field is being scrubbed).
fn model_fields(ui: &mut Ui, c: Color) -> Option<(Color, bool)> {
    let model_id = Id::new("cp_model");
    let model: usize = ui.data(|d| d.get_temp(model_id)).unwrap_or(0).min(MODELS.len() - 1);
    let mut out: Option<Color> = None;
    let mut done = true;
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        let resp = widgets::dropdown(ui, MODELS.get(model).copied().unwrap_or("Hex"), 52.0, None);
        crate::right::menu_popup(&resp).show(|ui| {
            for (i, m) in MODELS.iter().enumerate() {
                if crate::right::menu_item(ui, m, "") {
                    ui.data_mut(|d| d.insert_temp(model_id, i));
                    ui.close();
                }
            }
        });
        let num = |ui: &mut Ui, id: &str, v: f64, w: f32| match widgets::number_field(
            ui,
            Id::new(("cp_f", id)),
            widgets::Lead::None,
            Some(v.round()),
            "",
            w,
            1.0,
        ) {
            Some(widgets::FieldEdit::Set(x)) => Some((x, true)),
            Some(widgets::FieldEdit::Scrub(x, end)) => Some((x, end)),
            None => None,
        };
        match MODELS.get(model).copied().unwrap_or("Hex") {
            "RGB" => {
                let [r, g, b, _] = c.to_rgba8();
                let vals = [f64::from(r), f64::from(g), f64::from(b)];
                for (k, v) in vals.iter().enumerate() {
                    if let Some((x, end)) = num(ui, ["r", "g", "b"][k.min(2)], *v, 31.0) {
                        done = end;
                        let mut rgb = vals;
                        if let Some(slot) = rgb.get_mut(k) {
                            *slot = x.clamp(0.0, 255.0);
                        }
                        out = Some(Color::new(rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0, c.a));
                    }
                }
            }
            "HSL" | "HSB" => {
                let hsl = model == 2;
                let (h, s2, l2) = if hsl {
                    let x = figcraft_color::Hsl::from_color(c);
                    (x.h, x.s, x.l)
                } else {
                    let x = Hsb::from_color(c);
                    (x.h, x.s, x.b)
                };
                let vals = [f64::from(h), f64::from(s2) * 100.0, f64::from(l2) * 100.0];
                for (k, v) in vals.iter().enumerate() {
                    if let Some((x, end)) = num(ui, ["h", "s", "l"][k.min(2)], *v, 31.0) {
                        done = end;
                        let mut v3 = vals;
                        if let Some(slot) = v3.get_mut(k) {
                            *slot = if k == 0 { x.rem_euclid(360.0) } else { x.clamp(0.0, 100.0) };
                        }
                        let (h, a, b) = (v3[0] as f32, v3[1] as f32 / 100.0, v3[2] as f32 / 100.0);
                        let col = if hsl { figcraft_color::Hsl { h, s: a, l: b }.to_color() } else { Hsb { h, s: a, b }.to_color() };
                        out = Some(col.with_alpha(c.a));
                    }
                }
            }
            "CSS" => {
                if let Some(text) = widgets::text_field(ui, Id::new("cp_css"), &c.to_css(), 152.0, "") {
                    out = Color::parse_css(&text);
                }
            }
            _ => {
                if let Some(h) = widgets::text_field(ui, Id::new("cp_hex"), &c.to_hex(), 101.0, "") {
                    out = Color::from_hex_or_name(&h).map(|h| h.with_alpha(if h.a < 1.0 { h.a } else { c.a }));
                }
            }
        }
        if MODELS.get(model) != Some(&"CSS") {
            let alpha = Some((f64::from(out.unwrap_or(c).a) * 100.0).round());
            let edit = match widgets::number_field(ui, Id::new("cp_a"), widgets::Lead::None, alpha, "%", 44.0, 1.0) {
                Some(widgets::FieldEdit::Set(v)) => Some((v, true)),
                Some(widgets::FieldEdit::Scrub(v, end)) => Some((v, end)),
                None => None,
            };
            if let Some((v, end)) = edit {
                done = end;
                out = Some(out.unwrap_or(c).with_alpha((v / 100.0).clamp(0.0, 1.0) as f32));
            }
        }
    });
    out.map(|c| (c, done))
}

fn hue_color(ctx: &egui::Context, h: f32) -> Color32 {
    doc_c32(ctx, Hsb { h, s: 1.0, b: 1.0 }.to_color())
}

fn sv_square(ui: &mut Ui, rect: Rect, hsb: &mut Hsb) -> (bool, bool) {
    // White → hue horizontally, then transparent → black vertically.
    let mut m = Mesh::default();
    let hc = hue_color(ui.ctx(), hsb.h);
    let corners = [(rect.left_top(), Color32::WHITE), (rect.right_top(), hc), (rect.right_bottom(), hc), (rect.left_bottom(), Color32::WHITE)];
    for (p, c) in corners {
        m.colored_vertex(p, c);
    }
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    ui.painter().add(m);
    let mut m2 = Mesh::default();
    for (p, c) in [
        (rect.left_top(), Color32::TRANSPARENT),
        (rect.right_top(), Color32::TRANSPARENT),
        (rect.right_bottom(), Color32::BLACK),
        (rect.left_bottom(), Color32::BLACK),
    ] {
        m2.colored_vertex(p, c);
    }
    m2.add_triangle(0, 1, 2);
    m2.add_triangle(0, 2, 3);
    ui.painter().add(m2);
    let resp = ui.interact(rect, Id::new("cp_sv"), Sense::click_and_drag());
    let mut changed = false;
    if (resp.dragged() || resp.clicked() || resp.drag_started())
        && let Some(p) = resp.interact_pointer_pos()
    {
        hsb.s = ((p.x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
        hsb.b = 1.0 - ((p.y - rect.min.y) / rect.height()).clamp(0.0, 1.0);
        changed = true;
    }
    let knob = pos2(rect.min.x + hsb.s * rect.width(), rect.min.y + (1.0 - hsb.b) * rect.height());
    ui.painter().circle_stroke(knob, 6.0, Stroke::new(2.0, Color32::WHITE));
    ui.painter().circle_stroke(knob, 7.0, Stroke::new(1.0, Color32::from_black_alpha(80)));
    (changed, resp.drag_stopped() || resp.clicked())
}

fn hue_slider(ui: &mut Ui, rect: Rect, hsb: &mut Hsb) -> (bool, bool) {
    let n = 12;
    let mut m = Mesh::default();
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let x = rect.min.x + t * rect.width();
        let c = hue_color(ui.ctx(), t * 360.0);
        m.colored_vertex(pos2(x, rect.min.y), c);
        m.colored_vertex(pos2(x, rect.max.y), c);
        if i > 0 {
            let k = (i * 2) as u32;
            m.add_triangle(k - 2, k - 1, k);
            m.add_triangle(k - 1, k, k + 1);
        }
    }
    ui.painter().add(m);
    slider_knob(ui, rect, hsb.h / 360.0, Id::new("cp_hue"))
        .map(|(t, done)| {
            hsb.h = (t * 360.0).clamp(0.0, 359.9);
            (true, done)
        })
        .unwrap_or((false, false))
}

fn slider_knob(ui: &mut Ui, rect: Rect, t: f32, id: Id) -> Option<(f32, bool)> {
    let resp = ui.interact(rect.expand(4.0), id, Sense::click_and_drag());
    let x = rect.min.x + t.clamp(0.0, 1.0) * rect.width();
    ui.painter().circle_stroke(pos2(x, rect.center().y), rect.height() / 2.0 + 1.0, Stroke::new(2.0, Color32::WHITE));
    ui.painter().circle_stroke(pos2(x, rect.center().y), rect.height() / 2.0 + 2.0, Stroke::new(1.0, Color32::from_black_alpha(80)));
    if (resp.dragged() || resp.clicked() || resp.drag_started())
        && let Some(p) = resp.interact_pointer_pos()
    {
        return Some((((p.x - rect.min.x) / rect.width()).clamp(0.0, 1.0), resp.drag_stopped() || resp.clicked()));
    }
    if resp.drag_stopped() {
        return Some((t, true));
    }
    None
}

fn alpha_slider(ui: &mut Ui, rect: Rect, color: Color) -> Option<(f32, bool)> {
    widgets::checker(&ui.painter().with_clip_rect(rect), rect, rect.height() / 2.0);
    let mut m = Mesh::default();
    let opaque = doc_c32(ui.ctx(), color.with_alpha(1.0));
    for (p, c) in [
        (rect.left_top(), Color32::TRANSPARENT),
        (rect.right_top(), opaque),
        (rect.right_bottom(), opaque),
        (rect.left_bottom(), Color32::TRANSPARENT),
    ] {
        m.colored_vertex(p, c);
    }
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    ui.painter().add(m);
    slider_knob(ui, rect, color.a, Id::new("cp_alpha"))
}

pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    let Some(target) = app.ui.picker.clone() else { return };
    // A layer's picker belongs to the selection it was opened for: once that changes, close it
    // rather than keep editing layers that are no longer selected.
    let stale = match &target {
        PickerTarget::Paint { ids, .. } | PickerTarget::Effect { ids, .. } => *ids != app.session.selection(),
        PickerTarget::TextRange { id, .. } => !app.session.selection().contains(id),
        _ => false,
    };
    let current = if stale { None } else { current_paint(app, &target) };
    let Some(mut paint) = current else {
        app.ui.picker = None;
        return;
    };
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    // A plugin's colour field: beside its window, which sits where the picker otherwise would.
    let beside = match &target {
        PickerTarget::PluginField { plugin, .. } => crate::plugins_ui::field_window(app, ctx, plugin).map(|r| pos2(r.min.x - 248.0, r.min.y)),
        _ => None,
    };
    let pos = beside.unwrap_or(pos2(screen.max.x - app.ui.right_width - 260.0, 140.0));
    let mut open = true;
    let mut close_now = false;
    egui::Window::new("color_picker")
        .title_bar(false)
        .fixed_pos(pos)
        .fixed_size(vec2(240.0, 0.0))
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(0)))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            // Header.
            let (hr, _) = ui.allocate_exact_size(vec2(240.0, 40.0), Sense::hover());
            ui.painter().text(pos2(hr.min.x + 16.0, hr.center().y), Align2::LEFT_CENTER, tr("Custom"), semibold(11.0), t.text);
            let cr = Rect::from_center_size(pos2(hr.max.x - 20.0, hr.center().y), vec2(24.0, 24.0));
            let cresp = ui.interact(cr, Id::new("cp_close"), Sense::click());
            icons::paint(ui.painter(), cr.shrink(6.0), Icon::Close, t.icon);
            if cresp.clicked() {
                close_now = true;
            }
            // Paint type.
            if !matches!(
                target,
                PickerTarget::Effect { .. } | PickerTarget::PageBackground | PickerTarget::Variable { .. } | PickerTarget::PluginField { .. }
            ) {
                let kinds = [
                    (Icon::Fill, tr("Solid")),
                    (Icon::GradientLinear, tr("Linear")),
                    (Icon::GradientRadial, tr("Radial")),
                    (Icon::GradientAngular, tr("Angular")),
                    (Icon::GradientDiamond, tr("Diamond")),
                    (Icon::Image, tr("Image")),
                    (Icon::ViewGrid, tr("Pattern")),
                ];
                let active = match &paint.kind {
                    PaintKind::Solid { .. } => 0,
                    PaintKind::Gradient(g) => match g.kind {
                        GradientKind::Linear => 1,
                        GradientKind::Radial => 2,
                        GradientKind::Angular => 3,
                        GradientKind::Diamond => 4,
                    },
                    PaintKind::Image(_) => 5,
                    PaintKind::Pattern(_) => 6,
                };
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    if let Some(i) = widgets::segmented(ui, &kinds, Some(active), 30.0) {
                        let base = paint.preview_color();
                        let mut image_pending = false;
                        paint.kind = match i {
                            0 => PaintKind::Solid { color: base.with_alpha(1.0) },
                            5 => {
                                image_pending = true;
                                let target = target.clone();
                                app.pick_open(tr("Choose image"), &["png", "jpg", "jpeg", "webp", "gif", "tif", "tiff"], move |app, p| {
                                    let Ok(bytes) = std::fs::read(&p) else { return };
                                    let Some(hash) = app.session.doc_mut().ok().and_then(|d| figcraft_format_add(d, bytes)) else { return };
                                    if let Some(mut paint) = current_paint(app, &target) {
                                        paint.kind = Paint::image(hash, ImageScaleMode::Fill).kind;
                                        write(app, &target, &paint, false);
                                    }
                                });
                                paint.kind.clone()
                            }
                            6 => match pattern_sources(app, &target).first() {
                                Some((id, _)) => Paint::pattern(id.0).kind,
                                None => {
                                    app.toast(tr("Add a layer or component to tile first").to_string());
                                    paint.kind.clone()
                                }
                            },
                            k => {
                                let kind = [GradientKind::Linear, GradientKind::Radial, GradientKind::Angular, GradientKind::Diamond][k - 1];
                                match &paint.kind {
                                    PaintKind::Gradient(g) => {
                                        PaintKind::Gradient(Gradient { kind, ..Gradient::default_for(kind, base).with_stops(g.stops.clone()) })
                                    }
                                    _ => PaintKind::Gradient(Gradient::default_for(kind, base)),
                                }
                            }
                        };
                        if !image_pending {
                            write(app, &target, &paint, false);
                        }
                    }
                });
            }
            // The paint's own blend mode, with the paints and layers under it.
            if !matches!(
                target,
                PickerTarget::Effect { .. } | PickerTarget::PageBackground | PickerTarget::Variable { .. } | PickerTarget::PluginField { .. }
            ) {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    let resp = widgets::dropdown(ui, tr(paint.blend.label()), 208.0, Some(Icon::Blend)).on_hover_text(tr("Blend mode"));
                    crate::right::menu_popup(&resp).width(200.0).show(|ui| {
                        if let Some(b) = crate::right::blend_menu(ui, false) {
                            paint.blend = b;
                            write(app, &target, &paint, false);
                        }
                    });
                });
            }
            // Gradient stops.
            let mut edit_color: Option<Color> = match &paint.kind {
                PaintKind::Solid { color } => Some(color.with_alpha(color.a * paint.opacity)),
                _ => None,
            };
            let sel_stop_id = stop_id();
            if let PaintKind::Gradient(g) = &paint.kind {
                let mut g = g.clone();
                let mut sel: usize = ui.data(|d| d.get_temp(sel_stop_id)).unwrap_or(0).min(g.stops.len().saturating_sub(1));
                let (bar, _) = ui.allocate_exact_size(vec2(240.0, 28.0), Sense::hover());
                let bar = bar.shrink2(vec2(16.0, 6.0));
                widgets::checker(&ui.painter().with_clip_rect(bar), bar, 4.0);
                let n = 24;
                for i in 0..n {
                    let a = i as f32 / n as f32;
                    let b = (i + 1) as f32 / n as f32;
                    let r = Rect::from_min_max(pos2(bar.min.x + a * bar.width(), bar.min.y), pos2(bar.min.x + b * bar.width(), bar.max.y));
                    ui.painter().rect_filled(r, CornerRadius::ZERO, doc_c32(ui.ctx(), g.sample((a + b) / 2.0)));
                }
                let mut changed = false;
                let mut done = false;
                // The bar first, so the stops on it take their own clicks.
                let bresp = ui.interact(bar, Id::new("cp_bar"), Sense::click());
                for (i, s) in g.stops.clone().iter().enumerate() {
                    let x = bar.min.x + s.position * bar.width();
                    let r = Rect::from_center_size(pos2(x, bar.center().y), vec2(12.0, 18.0));
                    let resp = ui.interact(r, Id::new(("cp_stopk", i)), Sense::click_and_drag());
                    ui.painter().rect_filled(r, CornerRadius::same(3), doc_c32(ui.ctx(), s.color.with_alpha(1.0)));
                    ui.painter().rect_stroke(
                        r,
                        CornerRadius::same(3),
                        Stroke::new(if i == sel { 2.0 } else { 1.0 }, if i == sel { crate::theme::ACCENT } else { Color32::WHITE }),
                        StrokeKind::Outside,
                    );
                    if resp.clicked() || resp.drag_started() {
                        sel = i;
                    }
                    if resp.dragged()
                        && let Some(p) = resp.interact_pointer_pos()
                        && let Some(st) = g.stops.get_mut(i)
                    {
                        st.position = ((p.x - bar.min.x) / bar.width()).clamp(0.0, 1.0);
                        changed = true;
                    }
                    if resp.drag_stopped() {
                        done = true;
                    }
                }
                // Click on the bar adds a stop.
                if bresp.clicked()
                    && let Some(p) = bresp.interact_pointer_pos()
                {
                    let pos = ((p.x - bar.min.x) / bar.width()).clamp(0.0, 1.0);
                    g.stops.push(ColorStop { position: pos, color: g.sample(pos) });
                    sel = g.stops.len() - 1;
                    changed = true;
                    done = true;
                }
                ui.data_mut(|d| d.insert_temp(sel_stop_id, sel));
                if changed {
                    paint.kind = PaintKind::Gradient(g.clone());
                    write(app, &target, &paint, !done);
                }
                if done {
                    finish(app);
                }
                edit_color = g.stops.get(sel).map(|s| s.color);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    if ui.small_button(tr("Remove stop")).clicked() && g.stops.len() > 2 {
                        g.stops.remove(sel);
                        ui.data_mut(|d| d.insert_temp(sel_stop_id, 0usize));
                        paint.kind = PaintKind::Gradient(g.clone());
                        write(app, &target, &paint, false);
                    }
                    if ui.small_button(tr("Flip")).clicked() {
                        for s in &mut g.stops {
                            s.position = 1.0 - s.position;
                        }
                        paint.kind = PaintKind::Gradient(g.clone());
                        write(app, &target, &paint, false);
                    }
                    if ui.small_button(tr("Rotate 90°")).clicked() {
                        let (fx, fy) = g.from;
                        let rot = |(x, y): (f64, f64)| (fx - (y - fy), fy + (x - fx));
                        g.to = rot(g.to);
                        g.width = rot(g.width);
                        paint.kind = PaintKind::Gradient(g.clone());
                        write(app, &target, &paint, false);
                    }
                });
                // The selected stop's colour can come from a colour variable.
                if let PickerTarget::Paint { fills, index, ids } = &target {
                    let field = format!("{}/{index}/stops/{sel}/color", if *fills { "fills" } else { "strokes" });
                    let stop = g.stops.get(sel).map(|s| s.color);
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        crate::variable_ui::bindable(app, ui, ids, &field, figcraft_doc::VariableType::Color, 208.0, |_, ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(208.0, widgets::FIELD_H), Sense::hover());
                            ui.painter().rect_filled(r, CornerRadius::same(5), t.field);
                            let sw = Rect::from_center_size(pos2(r.min.x + 13.0, r.center().y), vec2(14.0, 14.0));
                            if let Some(c) = stop {
                                widgets::paint_swatch(ui.painter(), sw, &Paint::solid(c));
                                let text = trf("Stop {n}  {hex}", &[("n", &(sel + 1)), ("hex", &c.to_hex())]);
                                ui.painter().text(pos2(sw.max.x + 8.0, r.center().y), Align2::LEFT_CENTER, text, regular(11.0), t.text);
                            }
                        });
                    });
                }
            }
            if let PaintKind::Image(img) = &paint.kind {
                let mut img = img.clone();
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    let resp = widgets::dropdown(ui, tr(&format!("{:?}", img.scale_mode)), 100.0, None);
                    crate::right::menu_popup(&resp).show(|ui| {
                        for (l, m) in [
                            (tr("Fill"), ImageScaleMode::Fill),
                            (tr("Fit"), ImageScaleMode::Fit),
                            (tr("Crop"), ImageScaleMode::Crop),
                            (tr("Tile"), ImageScaleMode::Tile),
                        ] {
                            if crate::right::menu_item(ui, l, "") {
                                img.scale_mode = m;
                                paint.kind = PaintKind::Image(img.clone());
                                write(app, &target, &paint, false);
                                ui.close();
                            }
                        }
                    });
                    if icons::button(ui, Icon::Rotate, 24.0, false, tr("Rotate 90°")).clicked() {
                        img.rotation = (img.rotation + 1) % 4;
                        paint.kind = PaintKind::Image(img.clone());
                        write(app, &target, &paint, false);
                    }
                });
                // The image's own colour profile (decoding converts it into the file's).
                let source = app.session.doc().ok().and_then(|d| figcraft_render::images::profile_name(&img.image, d.images.get(&img.image)));
                let source = match source.as_deref() {
                    None => tr("Unknown — treated as sRGB"),
                    Some("") => tr("Embedded profile"),
                    Some(name) => name,
                };
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    let text = trf("Color profile: {name}", &[("name", &source)]);
                    ui.add(egui::Label::new(egui::RichText::new(text).font(regular(11.0)).color(t.text_secondary)).truncate());
                });
                image_presets(app, ui, &target, &mut paint, img.filters);
                for (label, val) in [
                    (tr_noop("Exposure"), img.filters.exposure),
                    (tr_noop("Contrast"), img.filters.contrast),
                    (tr_noop("Saturation"), img.filters.saturation),
                    (tr_noop("Temperature"), img.filters.temperature),
                    (tr_noop("Tint"), img.filters.tint),
                    (tr_noop("Highlights"), img.filters.highlights),
                    (tr_noop("Shadows"), img.filters.shadows),
                ] {
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        // Labels share a column so the sliders line up.
                        let (lr, _) = ui.allocate_exact_size(vec2(72.0, 18.0), Sense::hover());
                        ui.painter().text(lr.left_center(), Align2::LEFT_CENTER, tr(label), regular(11.0), t.text);
                        let mut v = val;
                        let r = ui.add(egui::Slider::new(&mut v, -1.0..=1.0).show_value(false));
                        if r.changed() {
                            let f = &mut img.filters;
                            match label {
                                "Exposure" => f.exposure = v,
                                "Contrast" => f.contrast = v,
                                "Saturation" => f.saturation = v,
                                "Temperature" => f.temperature = v,
                                "Tint" => f.tint = v,
                                "Highlights" => f.highlights = v,
                                _ => f.shadows = v,
                            }
                            paint.kind = PaintKind::Image(img.clone());
                            write(app, &target, &paint, !r.drag_stopped());
                        }
                        if r.drag_stopped() {
                            finish(app);
                        }
                    });
                }
            }
            if let PaintKind::Pattern(pt) = paint.kind.clone() {
                pattern_controls(app, ui, &target, &mut paint, pt);
            }
            if let Some(color) = edit_color {
                let mut hsb = ui
                    .data(|d| d.get_temp::<(f32, f32, f32, [u8; 4])>(Id::new("cp_hsb")))
                    .filter(|(_, _, _, rgba)| *rgba == color.to_rgba8())
                    .map(|(h, s, b, _)| Hsb { h, s, b })
                    .unwrap_or_else(|| Hsb::from_color(color));
                let (sq, _) = ui.allocate_exact_size(vec2(240.0, 240.0), Sense::hover());
                let (c1, d1) = sv_square(ui, sq, &mut hsb);
                let (hr, _) = ui.allocate_exact_size(vec2(240.0, 28.0), Sense::hover());
                let hr = Rect::from_min_max(pos2(hr.min.x + 48.0, hr.min.y + 8.0), pos2(hr.max.x - 16.0, hr.max.y - 8.0));
                if icons::button(
                    &mut ui.new_child(
                        egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(hr.min.x - 36.0, hr.center().y - 12.0), vec2(24.0, 24.0))),
                    ),
                    Icon::Eyedropper,
                    24.0,
                    false,
                    tr("Pick color (I)"),
                )
                .clicked()
                {
                    app.ui.picker = None;
                    app.run("edit.pickColor", json!({}));
                    return;
                }
                let (c2, d2) = hue_slider(ui, hr, &mut hsb);
                let (ar, _) = ui.allocate_exact_size(vec2(240.0, 20.0), Sense::hover());
                let ar = Rect::from_min_max(pos2(ar.min.x + 48.0, ar.min.y + 2.0), pos2(ar.max.x - 16.0, ar.max.y - 6.0));
                let mut new_alpha = None;
                if let Some((a, d)) = alpha_slider(ui, ar, color) {
                    new_alpha = Some((a, d));
                }
                let mut nc = hsb.to_color().with_alpha(color.a);
                let mut done = d1 || d2;
                let mut changed = c1 || c2;
                if let Some((a, d)) = new_alpha {
                    nc = nc.with_alpha(a);
                    changed = true;
                    done |= d;
                }
                // Colour model fields (Hex, RGB, HSL, HSB or CSS) + alpha.
                if let Some((c, d)) = model_fields(ui, nc) {
                    nc = c;
                    hsb = Hsb::from_color(nc);
                    changed = true;
                    done |= d;
                }
                ui.data_mut(|d| d.insert_temp(Id::new("cp_hsb"), (hsb.h, hsb.s, hsb.b, nc.to_rgba8())));
                // A Display P3 colour sRGB can't show: the canvas (sRGB) shows the nearest one.
                if app.session.doc().is_ok_and(|d| d.color_profile.conversion_to(figcraft_color::ColorProfile::Srgb).clips(nc)) {
                    gamut_warning(ui, &t);
                }
                if changed {
                    match &mut paint.kind {
                        PaintKind::Solid { color } => {
                            *color = nc.with_alpha(1.0);
                            paint.opacity = nc.a;
                        }
                        PaintKind::Gradient(g) => {
                            let sel: usize = ui.data(|d| d.get_temp(sel_stop_id)).unwrap_or(0);
                            if let Some(s) = g.stops.get_mut(sel) {
                                s.color = nc;
                            }
                        }
                        PaintKind::Image(_) | PaintKind::Pattern(_) => {}
                    }
                    write(app, &target, &paint, !done);
                }
                if done {
                    finish(app);
                }
            }
            // Colours used on this page.
            let doc_colors = page_colors(app, ui);
            if !doc_colors.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    widgets::label(ui, tr("On this page"), t.text_secondary);
                });
                // Inset on both sides: `add_space` would only indent the first row of a wrap.
                inset(ui, |ui| {
                    for c in &doc_colors {
                        if widgets::swatch(ui, &Paint::solid(*c), 14.0).on_hover_text(c.to_hex()).clicked() {
                            set_color(app, &target, &mut paint, *c, sel_stop_id, ui);
                        }
                    }
                });
            }
            // Recent colours.
            let recent = app.session.recent_colors.clone();
            if !recent.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    widgets::label(ui, tr("Recent"), t.text_secondary);
                });
                inset(ui, |ui| {
                    for c in recent.iter().take(14) {
                        if widgets::swatch(ui, &Paint::solid(*c), 14.0).clicked() {
                            set_color(app, &target, &mut paint, *c, sel_stop_id, ui);
                        }
                    }
                });
            }
            ui.add_space(12.0);
        });
    if !open || close_now || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        finish(app);
        app.ui.picker = None;
    }
    let _ = Pos2::ZERO;
}

/// Swatches wrapped in rows inset 16 points from both sides of the picker.
fn inset(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(16, 0)).show(ui, |ui| ui.horizontal_wrapped(add));
}

/// The colour picker's note under the fields for a colour outside sRGB.
fn gamut_warning(ui: &mut Ui, t: &Tokens) {
    use crate::a11y::Tip;
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icons::paint(ui.painter(), r, Icon::GamutWarning, t.icon_secondary);
        widgets::label(ui, "Outside sRGB", t.text_secondary);
    })
    .response
    .tip("sRGB screens and files can't show this Display P3 color: the canvas shows the nearest sRGB color, P3 exports keep it");
}

/// Layers a pattern can tile: the file's components and the current page's top-level layers, but
/// the painted layers, what they contain and what contains them (at most 64).
fn pattern_sources(app: &FigApp, target: &PickerTarget) -> Vec<(NodeId, String)> {
    let Ok(doc) = app.session.doc() else { return Vec::new() };
    let painted = match target {
        PickerTarget::Paint { ids, .. } => ids.clone(),
        PickerTarget::TextRange { id, .. } => vec![*id],
        _ => Vec::new(),
    };
    let top = app.session.page().ok().and_then(|p| doc.node(p)).map(|p| p.children.clone()).unwrap_or_default();
    let comps = doc.nodes().filter(|n| matches!(n.kind, NodeKind::Component { .. })).map(|n| n.id);
    let mut out: Vec<(NodeId, String)> = Vec::new();
    for id in comps.chain(top) {
        let related = painted.iter().any(|t| doc.is_ancestor(*t, id) || doc.is_ancestor(id, *t));
        if !related
            && !out.iter().any(|(o, _)| *o == id)
            && let Some(n) = doc.node(id)
        {
            out.push((id, n.name.clone()));
        }
        if out.len() >= 64 {
            break;
        }
    }
    out
}

/// The pattern tab: source, tile type, alignment, scale and spacing.
fn pattern_controls(app: &mut FigApp, ui: &mut Ui, target: &PickerTarget, paint: &mut Paint, mut pt: PatternPaint) {
    use widgets::{FieldEdit, Lead};
    // Some(live): changed, as part of a drag when live.
    let mut changed: Option<bool> = None;
    let name = app.session.doc().ok().and_then(|d| d.node(NodeId(pt.source)).map(|n| n.name.clone())).unwrap_or_else(|| tr("Missing layer").into());
    let sources = pattern_sources(app, target);
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        let resp = widgets::dropdown(ui, &name, 208.0, None);
        crate::a11y::combo(&resp, tr("Pattern source"), &name, crate::a11y::menu_open(&resp));
        crate::right::menu_popup(&resp).show(|ui| {
            for (id, label) in &sources {
                if crate::right::menu_check(ui, id.0 == pt.source, label, "") {
                    pt.source = id.0;
                    changed = Some(false);
                    ui.close();
                }
            }
        });
    });
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        let tiles =
            [(tr("Grid"), PatternTile::Grid), (tr("Horizontal hex"), PatternTile::HorizontalHex), (tr("Vertical hex"), PatternTile::VerticalHex)];
        let label = tiles.iter().find(|t| t.1 == pt.tile).map_or(tr("Grid"), |t| t.0);
        let resp = widgets::dropdown(ui, label, 120.0, Some(Icon::ViewGrid));
        crate::a11y::combo(&resp, tr("Tile type"), label, crate::a11y::menu_open(&resp));
        crate::right::menu_popup(&resp).show(|ui| {
            for (l, v) in tiles {
                if crate::right::menu_check(ui, v == pt.tile, l, "") {
                    pt.tile = v;
                    changed = Some(false);
                    ui.close();
                }
            }
        });
        let aligns = [(tr("Start"), PatternAlign::Start), (tr("Center"), PatternAlign::Center), (tr("End"), PatternAlign::End)];
        let label = aligns.iter().find(|a| a.1 == pt.align).map_or(tr("Start"), |a| a.0);
        let resp = widgets::dropdown(ui, label, 80.0, None);
        crate::a11y::combo(&resp, tr("Pattern alignment"), label, crate::a11y::menu_open(&resp));
        crate::right::menu_popup(&resp).show(|ui| {
            for (l, v) in aligns {
                if crate::right::menu_check(ui, v == pt.align, l, "") {
                    pt.align = v;
                    changed = Some(false);
                    ui.close();
                }
            }
        });
    });
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        let field = |ui: &mut Ui, key: &str, lead: Lead, value: f64, suffix: &str| match widgets::number_field(
            ui,
            Id::new(key),
            lead,
            Some(value),
            suffix,
            64.0,
            1.0,
        ) {
            Some(FieldEdit::Set(v)) => Some((v, false)),
            Some(FieldEdit::Scrub(v, end)) => Some((v, !end)),
            None => None,
        };
        if let Some((v, live)) = field(ui, "cp_pattern_scale", Lead::Icon(Icon::Scale, tr("Pattern scale")), pt.scale * 100.0, "%") {
            let (lo, hi) = PatternPaint::SCALE_RANGE;
            pt.scale = (v / 100.0).clamp(lo, hi);
            changed = Some(live);
        }
        if let Some((v, live)) = field(ui, "cp_pattern_gap_x", Lead::Icon(Icon::PaddingH, tr("Horizontal spacing")), pt.spacing.0, "") {
            pt.spacing.0 = v.clamp(0.0, PatternPaint::MAX_SPACING);
            changed = Some(live);
        }
        if let Some((v, live)) = field(ui, "cp_pattern_gap_y", Lead::Icon(Icon::PaddingV, tr("Vertical spacing")), pt.spacing.1, "") {
            pt.spacing.1 = v.clamp(0.0, PatternPaint::MAX_SPACING);
            changed = Some(live);
        }
    });
    if let Some(live) = changed {
        paint.kind = PaintKind::Pattern(pt);
        write(app, target, paint, live);
        if !live {
            finish(app);
        }
    }
}

/// What the image preset row was asked to do.
enum PresetAction {
    Apply(String),
    Delete(String),
}

/// A saved preset in the presets menu: click applies it, the trailing button deletes it.
fn saved_preset_row(ui: &mut Ui, name: &str) -> Option<PresetAction> {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width().max(180.0);
    let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    let del = Rect::from_center_size(pos2(r.max.x - 14.0, r.center().y), vec2(20.0, 20.0));
    let del_resp = ui.interact(del, ui.id().with(("cp_preset_del", name)), Sense::click()).tip(tr("Delete preset"));
    let hot = resp.hovered() || del_resp.hovered();
    if hot {
        ui.painter().rect_filled(r, CornerRadius::same(5), crate::theme::ACCENT);
    }
    let col = if hot { Color32::WHITE } else { t.text };
    ui.painter().text(pos2(r.min.x + 8.0, r.center().y), Align2::LEFT_CENTER, name, regular(11.5), col);
    if hot {
        icons::paint(ui.painter(), del.shrink(4.0), Icon::Close, col);
    }
    if del_resp.clicked() {
        Some(PresetAction::Delete(name.to_string()))
    } else if resp.clicked() {
        Some(PresetAction::Apply(name.to_string()))
    } else {
        None
    }
}

/// The image paint's adjustment presets: a menu of the built-in presets and the file's saved ones
/// (named after the one in effect), "Save as preset…" and Reset.
fn image_presets(app: &mut FigApp, ui: &mut Ui, target: &PickerTarget, paint: &mut Paint, filters: ImageFilters) {
    let saved: Vec<(String, ImageFilters)> =
        app.session.doc().map(|d| d.image_presets.iter().map(|p| (p.name.clone(), p.filters)).collect()).unwrap_or_default();
    let built_in = ImageFilters::PRESETS.iter().filter(|(id, ..)| *id != "reset");
    let current = built_in
        .clone()
        .map(|(_, label, f)| (tr(label).to_string(), *f))
        .chain(saved.iter().cloned())
        .find(|(_, f)| *f == filters)
        .map(|(name, _)| name)
        .unwrap_or_else(|| if filters.is_identity() { tr("No preset").into() } else { tr("Custom").into() });
    let name_id = Id::new("cp_preset_name");
    let mut naming: Option<String> = ui.data(|d| d.get_temp(name_id));
    let mut action = None;
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        let resp = widgets::dropdown(ui, &current, 152.0, None).on_hover_text(tr("Adjustment presets"));
        crate::right::menu_popup(&resp).show(|ui| {
            for (id, label, _) in built_in {
                if crate::right::menu_item(ui, tr(label), "") {
                    action = Some(PresetAction::Apply(id.to_string()));
                    ui.close();
                }
            }
            ui.separator();
            for (name, _) in &saved {
                if let Some(a) = saved_preset_row(ui, name) {
                    action = Some(a);
                    ui.close();
                }
            }
            if !saved.is_empty() {
                ui.separator();
            }
            if crate::right::menu_item(ui, tr("Save as preset…"), "") {
                naming = Some(format!("Preset {}", saved.len() + 1));
                ui.close();
            }
        });
        if widgets::button(ui, tr("Reset"), false, !filters.is_identity()).on_hover_text(tr("Reset adjustments")).clicked() {
            action = Some(PresetAction::Apply("reset".into()));
        }
    });
    if let Some(name) = naming.as_mut() {
        let mut save = false;
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            let r = ui.add(egui::TextEdit::singleline(name).font(regular(11.0)).desired_width(136.0));
            crate::widgets::select_all_on_focus(ui, &r, name);
            if !r.has_focus() && !crate::a11y::field_focused(ui.ctx()) {
                r.request_focus();
            }
            save = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            save |= widgets::button(ui, tr("Save"), true, !name.trim().is_empty()).clicked();
        });
        if save && !name.trim().is_empty() {
            app.run("image.savePreset", json!({"name": name.trim(), "filters": filters}));
            naming = None;
        }
    }
    ui.data_mut(|d| match &naming {
        Some(n) => {
            d.insert_temp(name_id, n.clone());
        }
        None => d.remove::<String>(name_id),
    });
    match action {
        Some(PresetAction::Delete(name)) => {
            app.run("image.deletePreset", json!({"name": name}));
        }
        Some(PresetAction::Apply(name)) => match target {
            // Layer paints go through the command; other targets (a text range, a style) get the
            // preset's values written like any edit.
            PickerTarget::Paint { fills, index, ids } => {
                app.run("image.adjust", json!({"ids": ids, "target": if *fills { "fill" } else { "stroke" }, "index": index, "preset": name}));
            }
            _ => {
                let preset = app.session.doc().ok().and_then(|d| figcraft_engine::cmd::image::find_preset(d, &name));
                if let (Some(f), PaintKind::Image(img)) = (preset, &mut paint.kind) {
                    img.filters = f;
                    write(app, target, paint, false);
                }
            }
        },
        None => {}
    }
}

fn figcraft_format_add(doc: &mut figcraft_doc::Document, bytes: Vec<u8>) -> Option<String> {
    // Images are keyed by content hash; the engine's image.place uses the same scheme.
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format().ok()?.into_dimensions().ok()?;
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut s = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut s);
        format!("img{:016x}{:08x}", s.finish(), bytes.len())
    };
    doc.images.insert(hash.clone(), figcraft_doc::ImageData { bytes: std::sync::Arc::new(bytes), width: w, height: h });
    Some(hash)
}

trait WithStops {
    fn with_stops(self, stops: Vec<ColorStop>) -> Self;
}

impl WithStops for Gradient {
    fn with_stops(mut self, stops: Vec<ColorStop>) -> Self {
        if stops.len() >= 2 {
            self.stops = stops;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers_drag_tests::Ui;
    use figcraft_engine::Session;

    #[test]
    fn dragging_records_only_the_final_colour_in_recent_colors() {
        let mut u = Ui::new(Session::new());
        let target = PickerTarget::PageBackground;
        u.app.ui.picker = Some(target.clone());
        for hex in ["#110000", "#220000", "#330000"] {
            let c = Color::from_hex(hex).unwrap();
            write(&mut u.app, &target, &Paint::solid(c), true);
        }
        assert!(u.app.session.recent_colors.is_empty(), "previews must not be recorded: {:?}", u.app.session.recent_colors);
        finish(&mut u.app);
        let last = Color::from_hex("#330000").unwrap();
        assert_eq!(u.app.session.recent_colors, vec![last]);
    }

    #[test]
    fn a_drag_released_with_a_final_write_records_one_colour() {
        let mut u = Ui::new(Session::new());
        let target = PickerTarget::PageBackground;
        u.app.ui.picker = Some(target.clone());
        // Not a whole 8-bit value: the page stores it rounded, so recording both would add two near-identical swatches.
        let c = Color::new(0.3, 0.0, 0.0, 1.0);
        write(&mut u.app, &target, &Paint::solid(Color::new(0.1, 0.0, 0.0, 1.0)), true);
        write(&mut u.app, &target, &Paint::solid(c), false);
        finish(&mut u.app);
        assert_eq!(u.app.session.recent_colors.len(), 1, "{:?}", u.app.session.recent_colors);
    }

    #[test]
    fn a_click_outside_a_drag_still_records_the_colour() {
        let mut u = Ui::new(Session::new());
        let target = PickerTarget::PageBackground;
        u.app.ui.picker = Some(target.clone());
        let c = Color::from_hex("#445566").unwrap();
        write(&mut u.app, &target, &Paint::solid(c), false);
        assert_eq!(u.app.session.recent_colors, vec![c]);
    }
}
