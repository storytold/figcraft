//! On-canvas editing handles that sit above the pointer tools: gradient handles (while a
//! gradient fill is open in the colour picker), image crop handles (while a Crop-mode image
//! fill is open), progressive blur handles (while a progressive blur's settings are open) and
//! auto layout padding/gap handles (item gaps, wrapped row gaps, grid column
//! and row gaps, with the grid's tracks outlined).

use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use figcraft_color::{ImagePaint, ImageScaleMode, Paint, PaintKind};
use figcraft_doc::{Blur, ContentAlign, Effect, LayoutMode, NodeId};
use figcraft_geom::Point;
use serde_json::json;

use crate::colorpicker::PickerTarget;
use crate::theme::{ACCENT, semibold};
use crate::{FigApp, RightTab};

pub(crate) const PINK: Color32 = Color32::from_rgb(0xF2, 0x48, 0x8C);

pub(crate) fn pink_label(painter: &egui::Painter, label: String, center: Pos2) {
    let galley = painter.layout_no_wrap(label, semibold(10.0), Color32::WHITE);
    let rect = Rect::from_center_size(center, galley.size() + vec2(8.0, 4.0));
    painter.rect_filled(rect, CornerRadius::same(3), PINK);
    painter.galley(rect.min + vec2(4.0, 2.0), galley, Color32::WHITE);
}

#[derive(Clone, Debug)]
enum Drag {
    /// Gradient handle 0 = from, 1 = to, 2 = width; 3 + i = stop `i` (slides along the line).
    Gradient { id: NodeId, fills: bool, index: usize, which: usize },
    /// Padding side 0 top, 1 right, 2 bottom, 3 left.
    Padding { id: NodeId, side: usize, start: Pos2, value: f64 },
    /// A gap band: `key` is the prop it edits (`gap` or `crossGap`), `vertical` the drag axis.
    Gap { id: NodeId, start: Pos2, value: f64, vertical: bool, key: &'static str },
    /// Progressive blur of effect `index`: 0 = start point, 1 = end point.
    Blur { id: NodeId, index: usize, which: usize },
    /// Image crop: corner 0..4 (tl, tr, br, bl) scales about the opposite corner, 4 moves.
    Crop { id: NodeId, fills: bool, index: usize, which: usize, start: (f64, f64), rect: (f64, f64, f64, f64) },
}

/// The Crop-mode image being edited: (node, fills, index, paint, image rect in the unit box).
type UnitRect = (f64, f64, f64, f64);

fn crop_target(app: &FigApp) -> Option<(NodeId, bool, usize, ImagePaint, UnitRect)> {
    let Some(PickerTarget::Paint { fills, index, ids }) = &app.ui.picker else { return None };
    if ids.len() != 1 {
        return None;
    }
    let id = *ids.first()?;
    let doc = app.session.doc().ok()?;
    let n = doc.node(id)?;
    let list = if *fills { &n.fills } else { &n.strokes };
    let PaintKind::Image(img) = &list.get(*index)?.kind else { return None };
    if img.scale_mode != ImageScaleMode::Crop {
        return None;
    }
    let data = doc.images.get(&img.image)?;
    let crop = img.effective_crop(f64::from(data.width), f64::from(data.height), n.width, n.height);
    Some((id, *fills, *index, img.clone(), ImagePaint::crop_rect(crop)))
}

fn crop_corners(app: &FigApp, id: NodeId, r: (f64, f64, f64, f64)) -> Option<[Pos2; 4]> {
    Some([
        unit_to_screen(app, id, (r.0, r.1))?,
        unit_to_screen(app, id, (r.2, r.1))?,
        unit_to_screen(app, id, (r.2, r.3))?,
        unit_to_screen(app, id, (r.0, r.3))?,
    ])
}

fn drag_id() -> Id {
    Id::new("figcraft_handle_drag")
}

/// The gradient being edited (single node, picker open on a gradient paint).
fn gradient_target(app: &FigApp) -> Option<(NodeId, bool, usize, figcraft_color::Gradient)> {
    let Some(PickerTarget::Paint { fills, index, ids }) = &app.ui.picker else { return None };
    let id = *ids.first()?;
    if ids.len() != 1 {
        return None;
    }
    let n = app.session.doc().ok()?.node(id)?;
    let list = if *fills { &n.fills } else { &n.strokes };
    match &list.get(*index)?.kind {
        PaintKind::Gradient(g) => Some((id, *fills, *index, g.clone())),
        _ => None,
    }
}

/// The progressive blur being edited: a single selected layer's first visible progressive blur
/// whose settings are open in the Design panel (layer index, effect).
fn blur_target(app: &FigApp, ctx: &egui::Context) -> Option<(NodeId, usize, Effect)> {
    if app.ui.right_tab != RightTab::Design {
        return None;
    }
    let sel = app.session.selection();
    let [id] = sel.as_slice() else { return None };
    let n = app.session.doc().ok()?.node(*id)?;
    let (i, e) =
        n.effects.iter().enumerate().find(|(i, e)| e.visible && e.blur().is_some_and(Blur::is_progressive) && crate::right::effect_open(ctx, *i))?;
    Some((*id, i, e.clone()))
}

/// A progressive blur's start and end points on screen.
fn blur_points(app: &FigApp, id: NodeId, e: &Effect) -> Option<[Pos2; 2]> {
    let b = e.blur()?.sanitized();
    Some([unit_to_screen(app, id, b.start)?, unit_to_screen(app, id, b.end)?])
}

/// Where `p` projects onto segment a→b: (t in 0..1, distance in points).
fn line_param(a: Pos2, b: Pos2, p: Pos2) -> Option<(f64, f32)> {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 < 1e-6 {
        return None;
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    Some((f64::from(t), (a + ab * t - p).length()))
}

fn set_gradient(app: &mut FigApp, id: NodeId, fills: bool, index: usize, g: figcraft_color::Gradient) {
    let Some(n) = app.session.doc().ok().and_then(|d| d.node(id)).cloned() else { return };
    let list = if fills { &n.fills } else { &n.strokes };
    let Some(mut paint) = list.get(index).cloned() else { return };
    paint.kind = PaintKind::Gradient(g);
    let cmd = if fills { "fill.set" } else { "stroke.set" };
    let _ = app.session.execute(cmd, &json!({"ids": [id], "index": index, "paint": paint}));
}

/// Delete/Backspace while a gradient is open: remove its selected stop (two always stay).
pub fn remove_selected_stop(app: &mut FigApp, ctx: &egui::Context) -> bool {
    let Some((id, fills, index, mut g)) = gradient_target(app) else { return false };
    let sel: usize = ctx.data(|d| d.get_temp(crate::colorpicker::stop_id())).unwrap_or(0);
    if g.stops.len() <= 2 || sel >= g.stops.len() {
        return g.stops.len() <= 2;
    }
    g.stops.remove(sel);
    ctx.data_mut(|d| d.insert_temp(crate::colorpicker::stop_id(), sel.saturating_sub(1)));
    let _ = app.session.begin_interaction("Remove gradient stop");
    set_gradient(app, id, fills, index, g);
    let _ = app.session.commit_interaction();
    true
}

fn unit_to_screen(app: &FigApp, id: NodeId, u: (f64, f64)) -> Option<Pos2> {
    let doc = app.session.doc().ok()?;
    let n = doc.node(id)?;
    let p = doc.world_transform(id) * Point::new(u.0 * n.width, u.1 * n.height);
    Some(app.to_screen(p.x, p.y))
}

fn screen_to_unit(app: &FigApp, id: NodeId, s: Pos2) -> Option<(f64, f64)> {
    let doc = app.session.doc().ok()?;
    let n = doc.node(id)?;
    let p = app.to_page(s);
    let l = doc.world_transform(id).inverse() * p;
    Some((l.x / n.width.max(1e-9), l.y / n.height.max(1e-9)))
}

/// A gap band between auto layout items (screen space): the prop it edits, its value and the
/// drag axis.
#[derive(Clone, Copy)]
struct GapZone {
    rect: Rect,
    key: &'static str,
    value: f64,
    vertical: bool,
}

/// Auto layout zones of a single selected AL frame, in screen space.
struct Zones {
    id: NodeId,
    /// Padding bands: (side, rect, value).
    bands: Vec<(usize, Rect, f64)>,
    gaps: Vec<GapZone>,
    /// Grid frames: the track boundaries to outline (screen-space segments).
    tracks: Vec<[Pos2; 2]>,
}

fn al_zones(app: &FigApp) -> Option<Zones> {
    let ds = app.session.ds().ok()?;
    if ds.selection.len() != 1 || ds.text_edit.is_some() || ds.vector_edit.is_some() {
        return None;
    }
    let id = *ds.selection.first()?;
    let doc = &ds.doc;
    let n = doc.node(id)?;
    let al = n.layout.as_ref()?;
    // Only for unrotated frames (axis-aligned bands).
    let w = doc.world_transform(id);
    if figcraft_geom::rotation_degrees(w).abs() > 0.01 {
        return None;
    }
    let to = |x: f64, y: f64| {
        let p = w * Point::new(x, y);
        app.to_screen(p.x, p.y)
    };
    let band = |x0: f64, y0: f64, x1: f64, y1: f64| Rect::from_two_pos(to(x0, y0), to(x1, y1));
    let [pt, pr, pb, pl] = al.padding;
    let (fw, fh) = (n.width, n.height);
    let bands = vec![
        (0, band(0.0, 0.0, fw, pt.max(4.0)), pt),
        (1, band(fw - pr.max(4.0), 0.0, fw, fh), pr),
        (2, band(0.0, fh - pb.max(4.0), fw, fh), pb),
        (3, band(0.0, 0.0, pl.max(4.0), fh), pl),
    ];
    // An auto gap shows (and is dragged from) the distance the layout gives it; the drag makes it a
    // fixed gap again, as typing one does.
    let (auto_main, auto_cross) =
        (al.main_align.is_auto_gap() && al.mode != LayoutMode::Grid, al.wrap && al.content_align == ContentAlign::SpaceBetween);
    let main = |rect: Rect, vertical: bool, dist: f64| GapZone { rect, key: "gap", value: if auto_main { dist } else { al.gap }, vertical };
    let cross =
        |rect: Rect, vertical: bool, dist: f64| GapZone { rect, key: "crossGap", value: if auto_cross { dist } else { al.cross_gap }, vertical };
    let mut gaps = Vec::new();
    let mut tracks = Vec::new();
    if let Some(g) = figcraft_layout::grid_geometry(doc, id) {
        // Grid: bands between columns (column gap) and between rows (row gap), and the tracks.
        let (top, bottom) = (g.rows.first().map(|r| r.0).unwrap_or(pt), g.rows.last().map(|r| r.1).unwrap_or(fh - pb));
        let (left, right) = (g.columns.first().map(|c| c.0).unwrap_or(pl), g.columns.last().map(|c| c.1).unwrap_or(fw - pr));
        for pair in g.columns.windows(2) {
            if let [a, b] = pair {
                gaps.push(main(band(a.1, top, b.0, bottom), false, al.gap));
            }
        }
        for pair in g.rows.windows(2) {
            if let [a, b] = pair {
                gaps.push(cross(band(left, a.1, right, b.0), true, al.cross_gap));
            }
        }
        for c in &g.columns {
            tracks.push([to(c.0, top), to(c.0, bottom)]);
            tracks.push([to(c.1, top), to(c.1, bottom)]);
        }
        for r in &g.rows {
            tracks.push([to(left, r.0), to(right, r.0)]);
            tracks.push([to(left, r.1), to(right, r.1)]);
        }
    } else {
        // Worked out along x: a vertical flow is a horizontal one turned on its side (`flip`
        // swaps the axes, `fband` swaps them back).
        let vertical = al.mode == LayoutMode::Vertical;
        let flip = |r: figcraft_geom::Rect| if vertical { figcraft_geom::Rect::new(r.y0, r.x0, r.y1, r.x1) } else { r };
        let fband = |x0: f64, y0: f64, x1: f64, y1: f64| if vertical { band(y0, x0, y1, x1) } else { band(x0, y0, x1, y1) };
        // Main-axis length and padding, cross-axis length and padding.
        let (mw, mp0, mp1, cw, cp0, cp1) = if vertical { (fh, pt, pb, fw, pl, pr) } else { (fw, pl, pr, fh, pt, pb) };
        let boxes: Vec<figcraft_geom::Rect> = figcraft_layout::flow_children(doc, id)
            .into_iter()
            .filter_map(|c| doc.node(c))
            .map(|c| flip(figcraft_geom::transform_rect_bbox(c.transform, c.local_rect())))
            .collect();
        // Lines: a wrapped item starts before the end of the previous one.
        let mut lines: Vec<Vec<figcraft_geom::Rect>> = Vec::new();
        for b in boxes {
            match lines.last_mut() {
                Some(line) if !al.wrap || line.last().is_some_and(|a| b.x0 >= a.x1 - 1e-6) => line.push(b),
                _ => lines.push(vec![b]),
            }
        }
        let span = |line: &[figcraft_geom::Rect]| line.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), r| (a.min(r.y0), b.max(r.y1)));
        for line in &lines {
            let (y0, y1) = if al.wrap { span(line) } else { (cp0, cw - cp1) };
            for pair in line.windows(2) {
                if let [ra, rb] = pair {
                    gaps.push(main(fband(ra.x1, y0, rb.x0, y1), vertical, rb.x0 - ra.x1));
                }
            }
        }
        for pair in lines.windows(2) {
            if let [a, b] = pair {
                gaps.push(cross(fband(mp0, span(a).1, mw - mp1, span(b).0), !vertical, span(b).0 - span(a).1));
            }
        }
    }
    gaps.retain(|g| g.rect.width() > 0.5 && g.rect.height() > 0.5);
    Some(Zones { id, bands, gaps, tracks })
}

/// Draw the handles.
pub fn draw(app: &FigApp, painter: &egui::Painter) {
    let hover = painter.ctx().input(|i| i.pointer.hover_pos());
    if let Some((id, _, _, g)) = gradient_target(app) {
        let (Some(a), Some(b), Some(c)) = (unit_to_screen(app, id, g.from), unit_to_screen(app, id, g.to), unit_to_screen(app, id, g.width)) else {
            return;
        };
        painter.line_segment([a, b], Stroke::new(2.0, Color32::WHITE));
        painter.line_segment([a, b], Stroke::new(1.0, Color32::from_black_alpha(120)));
        if g.kind != figcraft_color::GradientKind::Linear {
            painter.add(Shape::dashed_line(&[a, c], Stroke::new(1.0, Color32::WHITE), 4.0, 3.0));
        }
        let selected: usize = painter.ctx().data(|d| d.get_temp(crate::colorpicker::stop_id())).unwrap_or(0);
        for (i, s) in g.stops.iter().enumerate() {
            let p = a + (b - a) * s.position;
            let r = Rect::from_center_size(p, vec2(12.0, 12.0));
            painter.rect_filled(r, CornerRadius::same(2), crate::theme::doc_c32(painter.ctx(), s.color.with_alpha(1.0)));
            painter.rect_stroke(r, CornerRadius::same(2), Stroke::new(2.0, Color32::WHITE), StrokeKind::Outside);
            if i == selected {
                painter.rect_stroke(r.expand(2.0), CornerRadius::same(3), Stroke::new(1.5, ACCENT), StrokeKind::Outside);
            }
        }
        for (i, p) in [a, b, c].iter().enumerate() {
            if i == 2 && g.kind == figcraft_color::GradientKind::Linear {
                continue;
            }
            painter.circle_filled(*p, 5.0, Color32::WHITE);
            painter.circle_stroke(*p, 5.0, Stroke::new(1.5, ACCENT));
        }
        return;
    }
    if let Some((id, _, e)) = blur_target(app, painter.ctx()) {
        let Some([a, b]) = blur_points(app, id, &e) else { return };
        painter.line_segment([a, b], Stroke::new(2.0, Color32::WHITE));
        painter.line_segment([a, b], Stroke::new(1.0, Color32::from_black_alpha(120)));
        // The start is hollow (least blur), the end filled (most blur by default).
        painter.circle_filled(a, 5.0, Color32::WHITE);
        painter.circle_stroke(a, 5.0, Stroke::new(1.5, ACCENT));
        painter.circle_filled(b, 5.0, ACCENT);
        painter.circle_stroke(b, 5.0, Stroke::new(1.5, Color32::WHITE));
        return;
    }
    if let Some((id, _, _, _, r)) = crop_target(app) {
        let Some(c) = crop_corners(app, id, r) else { return };
        let outline = vec![c[0], c[1], c[2], c[3], c[0]];
        painter.add(Shape::line(outline.clone(), Stroke::new(2.0, Color32::WHITE)));
        painter.add(Shape::line(outline, Stroke::new(1.0, ACCENT)));
        for p in c {
            let hr = Rect::from_center_size(p, vec2(8.0, 8.0));
            painter.rect_filled(hr, CornerRadius::ZERO, Color32::WHITE);
            painter.rect_stroke(hr, CornerRadius::ZERO, Stroke::new(1.0, ACCENT), StrokeKind::Inside);
        }
        return;
    }
    let tool_move = app.session.tool.tool == figcraft_engine::ToolId::Move && !app.session.tool.busy();
    let dragging: Option<Drag> = painter.ctx().data(|d| d.get_temp(drag_id()));
    if !tool_move && dragging.is_none() {
        return;
    }
    let Some(z) = al_zones(app) else { return };
    let active_side = match &dragging {
        Some(Drag::Padding { side, .. }) => Some(*side),
        _ => None,
    };
    for seg in &z.tracks {
        painter.add(Shape::dashed_line(seg, Stroke::new(1.0, PINK.gamma_multiply(0.6)), 3.0, 3.0));
    }
    for (side, r, v) in &z.bands {
        let hot = hover.is_some_and(|h| r.contains(h)) || active_side == Some(*side);
        if hot {
            painter.rect_filled(*r, CornerRadius::ZERO, PINK.gamma_multiply(0.25));
            pink_label(painter, crate::canvas::fmt(*v), r.center());
        }
    }
    let drag_key = match &dragging {
        Some(Drag::Gap { key, .. }) => Some(*key),
        _ => None,
    };
    for g in &z.gaps {
        let r = g.rect;
        let hot = hover.is_some_and(|h| r.contains(h)) || drag_key == Some(g.key);
        if hot {
            painter.rect_filled(r, CornerRadius::ZERO, PINK.gamma_multiply(0.3));
            let c = r.center();
            painter.line_segment([pos2(c.x - 3.0, c.y), pos2(c.x + 3.0, c.y)], Stroke::new(1.0, PINK));
            pink_label(painter, crate::canvas::fmt(g.value), c);
        }
    }
    let _ = Align2::CENTER_CENTER;
}

/// Pointer handling; returns true when a handle owns the pointer this frame.
pub fn input(app: &mut FigApp, ctx: &egui::Context, pos: Pos2) -> bool {
    let (pressed, down, released, shift) = ctx.input(|i| {
        (
            i.pointer.button_pressed(egui::PointerButton::Primary),
            i.pointer.primary_down(),
            i.pointer.button_released(egui::PointerButton::Primary),
            i.modifiers.shift,
        )
    });
    let dragging: Option<Drag> = ctx.data(|d| d.get_temp(drag_id()));
    if let Some(drag) = dragging {
        if down {
            apply(app, &drag, pos, shift);
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if released || !down {
            let _ = app.session.commit_interaction();
            ctx.data_mut(|d| d.remove::<Drag>(drag_id()));
        }
        return true;
    }
    // Hover cursors / press to start.
    if let Some((id, fills, index, g)) = gradient_target(app) {
        let pts = [unit_to_screen(app, id, g.from), unit_to_screen(app, id, g.to), unit_to_screen(app, id, g.width)];
        for (which, p) in pts.iter().enumerate() {
            if which == 2 && g.kind == figcraft_color::GradientKind::Linear {
                continue;
            }
            if let Some(p) = p
                && (*p - pos).length() <= 8.0
            {
                ctx.set_cursor_icon(egui::CursorIcon::Grab);
                if pressed {
                    let _ = app.session.begin_interaction("Edit gradient");
                    ctx.data_mut(|d| d.insert_temp(drag_id(), Drag::Gradient { id, fills, index, which }));
                }
                return true;
            }
        }
        // Stops: select and slide them; a click elsewhere on the line adds one there.
        let (Some(a), Some(b)) = (pts[0], pts[1]) else { return false };
        let stop_hit = g.stops.iter().position(|st| (a + (b - a) * st.position - pos).length() <= 8.0);
        let t = line_param(a, b, pos);
        let on_line = t.is_some_and(|(_, d)| d <= 5.0);
        if stop_hit.is_none() && !on_line {
            return false;
        }
        ctx.set_cursor_icon(if stop_hit.is_some() { egui::CursorIcon::Grab } else { egui::CursorIcon::Copy });
        if pressed {
            let stop = match stop_hit {
                Some(i) => {
                    let _ = app.session.begin_interaction("Edit gradient");
                    i
                }
                None => {
                    let pos01 = t.map(|(t, _)| t).unwrap_or(0.5) as f32;
                    let mut g2 = g.clone();
                    g2.stops.push(figcraft_color::ColorStop { position: pos01, color: g.sample(pos01) });
                    let _ = app.session.begin_interaction("Add gradient stop");
                    set_gradient(app, id, fills, index, g2);
                    g.stops.len()
                }
            };
            ctx.data_mut(|d| d.insert_temp(crate::colorpicker::stop_id(), stop));
            ctx.data_mut(|d| d.insert_temp(drag_id(), Drag::Gradient { id, fills, index, which: 3 + stop }));
        }
        return true;
    }
    if let Some((id, index, e)) = blur_target(app, ctx) {
        let hit = blur_points(app, id, &e).and_then(|pts| pts.iter().position(|p| (*p - pos).length() <= 8.0));
        if let Some(which) = hit {
            ctx.set_cursor_icon(egui::CursorIcon::Grab);
            if pressed {
                let _ = app.session.begin_interaction("Edit progressive blur");
                ctx.data_mut(|d| d.insert_temp(drag_id(), Drag::Blur { id, index, which }));
            }
            return true;
        }
    }
    if let Some((id, fills, index, _, rect)) = crop_target(app) {
        let Some(c) = crop_corners(app, id, rect) else { return false };
        let start = screen_to_unit(app, id, pos).unwrap_or((0.0, 0.0));
        let corner = c.iter().position(|p| (*p - pos).length() <= 8.0);
        let inside = start.0 >= rect.0.min(rect.2) && start.0 <= rect.0.max(rect.2) && start.1 >= rect.1.min(rect.3) && start.1 <= rect.1.max(rect.3);
        let which = match corner {
            Some(i) => i,
            None if inside => 4,
            None => return false,
        };
        ctx.set_cursor_icon(if which == 4 {
            egui::CursorIcon::Move
        } else if which % 2 == 0 {
            egui::CursorIcon::ResizeNwSe
        } else {
            egui::CursorIcon::ResizeNeSw
        });
        if pressed {
            let _ = app.session.begin_interaction("Crop image");
            ctx.data_mut(|d| d.insert_temp(drag_id(), Drag::Crop { id, fills, index, which, start, rect }));
        }
        return true;
    }
    if app.session.tool.tool != figcraft_engine::ToolId::Move || app.session.tool.busy() {
        return false;
    }
    let Some(Zones { id, bands, gaps, .. }) = al_zones(app) else { return false };
    for (side, r, v) in bands {
        if r.contains(pos) {
            ctx.set_cursor_icon(if side % 2 == 0 { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal });
            if pressed {
                let _ = app.session.begin_interaction("Padding");
                ctx.data_mut(|d| d.insert_temp(drag_id(), Drag::Padding { id, side, start: pos, value: v }));
                return true;
            }
            return false;
        }
    }
    for g in gaps {
        if g.rect.contains(pos) {
            ctx.set_cursor_icon(if g.vertical { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal });
            if pressed {
                let _ = app.session.begin_interaction("Gap");
                ctx.data_mut(|d| d.insert_temp(drag_id(), Drag::Gap { id, start: pos, value: g.value, vertical: g.vertical, key: g.key }));
                return true;
            }
            return false;
        }
    }
    false
}

fn apply(app: &mut FigApp, drag: &Drag, pos: Pos2, shift: bool) {
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0).max(1e-6);
    match drag {
        Drag::Gradient { id, fills, index, which } => {
            let Some(u) = screen_to_unit(app, *id, pos) else { return };
            let Some(n) = app.session.doc().ok().and_then(|d| d.node(*id)).cloned() else { return };
            let list = if *fills { &n.fills } else { &n.strokes };
            let Some(mut paint) = list.get(*index).cloned() else { return };
            if let PaintKind::Gradient(g) = &mut paint.kind {
                match which {
                    0 => g.from = u,
                    1 => g.to = u,
                    2 => g.width = u,
                    stop => {
                        let (Some(a), Some(b)) = (unit_to_screen(app, *id, g.from), unit_to_screen(app, *id, g.to)) else { return };
                        if let (Some((t, _)), Some(st)) = (line_param(a, b, pos), g.stops.get_mut(stop - 3)) {
                            st.position = t as f32;
                        }
                    }
                }
                // An end of a linear gradient moved: its bands stay square to it.
                if *which < 2 && g.kind == figcraft_color::GradientKind::Linear {
                    g.square_linear_width(n.width, n.height);
                }
            }
            let cmd = if *fills { "fill.set" } else { "stroke.set" };
            let _ = app.session.execute(cmd, &json!({"ids": [id], "index": index, "paint": paint}));
            let _: Option<Paint> = None;
        }
        Drag::Blur { id, index, which } => {
            let Some(u) = screen_to_unit(app, *id, pos) else { return };
            let Some(mut e) = app.session.doc().ok().and_then(|d| d.node(*id)).and_then(|n| n.effects.get(*index)).cloned() else { return };
            if let Some(b) = e.blur_mut() {
                if *which == 0 {
                    b.start = u;
                } else {
                    b.end = u;
                }
            }
            let _ = app.session.execute("effect.set", &json!({"ids": [id], "index": index, "effect": e}));
        }
        Drag::Padding { id, side, start, value } => {
            let d = f64::from(match side {
                0 => pos.y - start.y,
                1 => start.x - pos.x,
                2 => start.y - pos.y,
                _ => pos.x - start.x,
            }) / zoom;
            let v = (value + d).round().max(0.0);
            let key = ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft"].get(*side).copied().unwrap_or("paddingTop");
            // Shift drags both sides of the axis together.
            let props =
                if shift { if side % 2 == 0 { json!({"paddingVertical": v}) } else { json!({"paddingHorizontal": v}) } } else { json!({ key: v }) };
            let _ = app.session.execute("node.set", &json!({"ids": [id], "props": props}));
        }
        Drag::Crop { id, fills, index, which, start, rect } => {
            let Some(u) = screen_to_unit(app, *id, pos) else { return };
            let (x0, y0, x1, y1) = *rect;
            let r = if *which == 4 {
                let (dx, dy) = (u.0 - start.0, u.1 - start.1);
                (x0 + dx, y0 + dy, x1 + dx, y1 + dy)
            } else {
                // Scale about the opposite corner, keeping the image's aspect ratio unless Shift.
                let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
                let (ax, ay) = corners.get((*which + 2) % 4).copied().unwrap_or((x0, y0));
                let (cx, cy) = corners.get(*which).copied().unwrap_or((x1, y1));
                let (ow, oh) = (cx - ax, cy - ay);
                let (mut nw, mut nh) = (u.0 - ax, u.1 - ay);
                if !shift {
                    // Node aspect matters: the unit box is not square in pixels.
                    let Some(n) = app.session.doc().ok().and_then(|d| d.node(*id)).cloned() else { return };
                    let k = ((nw * n.width).abs() / (ow * n.width).abs().max(1e-9)).max((nh * n.height).abs() / (oh * n.height).abs().max(1e-9));
                    nw = ow * k;
                    nh = oh * k;
                }
                if nw.abs() < 1e-3 || nh.abs() < 1e-3 || nw.signum() != ow.signum() || nh.signum() != oh.signum() {
                    return;
                }
                (ax.min(ax + nw), ay.min(ay + nh), ax.max(ax + nw), ay.max(ay + nh))
            };
            let Some(n) = app.session.doc().ok().and_then(|d| d.node(*id)).cloned() else { return };
            let list = if *fills { &n.fills } else { &n.strokes };
            let Some(mut paint) = list.get(*index).cloned() else { return };
            if let PaintKind::Image(img) = &mut paint.kind {
                img.crop = Some(ImagePaint::crop_from_rect(r.0, r.1, r.2, r.3));
            }
            let cmd = if *fills { "fill.set" } else { "stroke.set" };
            let _ = app.session.execute(cmd, &json!({"ids": [id], "index": index, "paint": paint}));
        }
        Drag::Gap { id, start, value, vertical, key } => {
            let d = f64::from(if *vertical { pos.y - start.y } else { pos.x - start.x }) / zoom;
            let v = (value + d).round().max(0.0);
            // Dragging an auto gap fixes it at what the drag makes of the distance it had.
            let mut props = json!({ *key: v });
            match (*key, app.session.doc().ok().and_then(|d| d.node(*id)).and_then(|n| n.layout.as_ref())) {
                ("crossGap", Some(l)) if l.content_align == ContentAlign::SpaceBetween => props["contentAlign"] = json!("auto"),
                ("gap", Some(l)) if l.main_align.is_auto_gap() && l.mode != LayoutMode::Grid => props["mainAlign"] = json!("min"),
                _ => {}
            }
            let _ = app.session.execute("node.set", &json!({"ids": [id], "props": props}));
        }
    }
}
