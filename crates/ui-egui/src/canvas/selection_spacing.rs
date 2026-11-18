//! Smart-selection fields and handles. Geometry and document edits belong to the engine.
use std::sync::Arc;

use egui::{CursorIcon, Id, Pos2, Rect, Stroke, Ui, vec2};
use figcraft_doc::{Document, NodeId};
use figcraft_engine::selection_reorder::ReorderLayout;
use figcraft_engine::selection_spacing::{Axis, SelectionSpacing, selection_spacing};
use serde_json::json;

use crate::FigApp;
use crate::handles::{PINK, pink_label};
use crate::i18n::tr;
use crate::icons::Icon;
use crate::widgets::{self, Lead};

#[derive(Clone)]
struct LayoutCache {
    document: u64,
    page: NodeId,
    doc: Arc<Document>,
    ids: Vec<NodeId>,
    frame: u64,
    layout: Arc<SelectionSpacing>,
    reorder: Option<Arc<ReorderLayout>>,
}

/// Return one cached spacing/reorder calculation per document, page, selection and frame,
/// refreshing next frame after an edit and reusing an unchanged document snapshot.
fn layout(app: &FigApp, ctx: &egui::Context, ids: &[NodeId]) -> Arc<SelectionSpacing> {
    let Ok(ds) = app.session.ds() else { return Arc::new(SelectionSpacing::default()) };
    // The gaps between layers moved together stay as they were: while a gesture only moves the
    // selection, they come from where it started.
    let doc = crate::selection_cache::moved(ds).filter(|_| ds.selection == ids).map_or_else(|| ds.doc.clone(), |(b, _)| b);
    let key = Id::unique("selection_spacing_layout");
    let frame = ctx.cumulative_frame_nr();
    if let Some(mut cache) = ctx.data(|d| d.get_temp::<LayoutCache>(key))
        && cache.document == ds.uid
        && cache.page == ds.page
        && cache.ids == ids
        && (cache.frame == frame || Arc::ptr_eq(&cache.doc, &doc))
    {
        if cache.frame != frame {
            cache.frame = frame;
            ctx.data_mut(|d| d.insert_temp(key, cache.clone()));
        }
        return cache.layout;
    }
    // Panel, input and drawing share one calculation per frame. If a command changes the
    // document during this frame, refresh next frame; unchanged snapshots also reuse it then.
    let layout = Arc::new(selection_spacing(&doc, ids));
    let reorder = ReorderLayout::new(&doc, ids).map(Arc::new);
    ctx.data_mut(|d| {
        d.insert_temp(key, LayoutCache { document: ds.uid, page: ds.page, doc, ids: ids.to_vec(), frame, layout: layout.clone(), reorder });
    });
    layout
}
/// Return the cached reorder layout, first ensuring the shared layout cache is populated.
fn reorder_layout(app: &FigApp, ctx: &egui::Context, ids: &[NodeId]) -> Option<Arc<ReorderLayout>> {
    layout(app, ctx, ids);
    ctx.data(|d| d.get_temp::<LayoutCache>(Id::unique("selection_spacing_layout"))).and_then(|c| c.reorder)
}

/// Stable egui id for spacing/reorder drag state.
fn drag_id() -> Id {
    Id::unique("selection_spacing_drag")
}

/// The reorder ring of a layer with world bounds `b`: its centre on screen and hit radius. The
/// radius is 8 px, at most a quarter of the layer's smaller side on screen, so most of a small
/// layer still drags the selection; a layer under 16 px on screen has no ring.
fn ring(app: &FigApp, b: figcraft_geom::Rect) -> Option<(Pos2, f32)> {
    let zoom = app.session.ds().map_or(1.0, |d| d.view.zoom);
    let side = ((b.x1 - b.x0).min(b.y1 - b.y0) * zoom) as f32;
    (side >= 16.0).then(|| (app.to_screen((b.x0 + b.x1) * 0.5, (b.y0 + b.y1) * 0.5), (side / 4.0).min(8.0)))
}

#[derive(Clone)]
enum Action {
    Spacing { axis: Axis, value: f64, last_value: Option<f64> },
    Reorder { layout: Arc<ReorderLayout>, source: NodeId, target: NodeId, swap: bool },
}

#[derive(Clone)]
struct Drag {
    ids: Vec<NodeId>,
    action: Action,
    start: Pos2,
    uid: u64,
    page: NodeId,
    moved: bool,
    /// The modifiers at the press: a release is a click only if it was a plain press, however the
    /// modifiers stand when the button comes up.
    plain: bool,
}

fn start_drag(app: &mut FigApp, ctx: &egui::Context, ids: Vec<NodeId>, p: Pos2, action: Action) {
    let label = match action {
        Action::Spacing { .. } => tr("Spacing"),
        Action::Reorder { .. } => tr("Reorder selection"),
    };
    let plain = ctx.input(|i| !(i.modifiers.shift || i.modifiers.alt || i.modifiers.command || i.modifiers.ctrl));
    if app.session.begin_interaction(label).is_ok()
        && let Ok(ds) = app.session.ds()
    {
        let drag = Drag { ids, action, start: p, uid: ds.uid, page: ds.page, moved: false, plain };
        app.ui.layers_focus = false;
        app.ui.pages_focus = None;
        app.ui.key_box = None;
        ctx.memory_mut(|m| {
            if let Some(f) = m.focused() {
                m.surrender_focus(f);
            }
        });
        ctx.data_mut(|d| d.insert_temp(drag_id(), drag));
    }
}

/// Draw horizontal and vertical spacing fields, running `arrange.spacing` with
/// `fromGesture`.
pub fn panel(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId]) {
    let layout = layout(app, ui.ctx(), ids);
    if layout.horizontal.is_none() && layout.vertical.is_none() {
        return;
    }
    widgets::sublabels(ui, tr("Spacing"), None);
    let col = widgets::col_width(ui.available_width());
    crate::right::row(ui, 32.0, |ui| {
        for (axis, key, icon, tip) in [
            (&layout.horizontal, "horizontal", Icon::DistributeH, tr("Horizontal spacing")),
            (&layout.vertical, "vertical", Icon::DistributeV, tr("Vertical spacing")),
        ] {
            let Some(axis) = axis else { continue };
            let edit = widgets::number_field(ui, Id::unique(("selection_spacing", key)), Lead::Icon(icon, tip), axis.value, "", col, 1.0);
            app.field_run(edit, "arrange.spacing", |v| json!({"ids":ids,key:v.clamp(-1e6,1e6),"fromGesture":true}));
        }
    });
}

#[derive(Clone)]
struct Handle {
    value: f64,
    center: Pos2,
    hit: Rect,
}

fn bounds(doc: &Document, ids: &[NodeId]) -> Option<figcraft_geom::Rect> {
    ids.iter().filter_map(|id| doc.world_bounds(*id)).reduce(|a, b| a.union(b))
}

fn handles(app: &FigApp, axis: &Axis) -> Vec<Handle> {
    let Ok(doc) = app.session.doc() else { return vec![] };
    let cross = |b: figcraft_geom::Rect| if axis.vertical { (b.x0, b.x1) } else { (b.y0, b.y1) };
    let mut out = Vec::new();
    for pair in axis.tracks.windows(2) {
        let Some((a, b)) = pair.first().zip(pair.get(1)) else { continue };
        let (Some(ab), Some(bb)) = (bounds(doc, &a.ids), bounds(doc, &b.ids)) else { continue };
        let gap = axis.start(bb) - axis.end(ab);
        let middle = (axis.start(bb) + axis.end(ab)) * 0.5;
        // Intersect neighbouring tracks' cross-axis spans in a sweep: one handle per row/cell,
        // without comparing every object in a large column to every object in the next one.
        let mut aa: Vec<_> = a.ids.iter().filter_map(|id| doc.world_bounds(*id)).map(cross).collect();
        let mut bs: Vec<_> = b.ids.iter().filter_map(|id| doc.world_bounds(*id)).map(cross).collect();
        aa.sort_by(|a, b| a.0.total_cmp(&b.0));
        bs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (mut i, mut j) = (0, 0);
        while let (Some(a), Some(b)) = (aa.get(i), bs.get(j)) {
            let (lo, hi) = (a.0.max(b.0), a.1.min(b.1));
            if hi > lo {
                let c = (lo + hi) * 0.5;
                let center = if axis.vertical { app.to_screen(c, middle) } else { app.to_screen(middle, c) };
                let size = if axis.vertical { vec2(24.0, 10.0) } else { vec2(10.0, 24.0) };
                out.push(Handle { value: gap, center, hit: Rect::from_center_size(center, size) });
            }
            if a.1 <= b.1 {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    out
}

fn available(app: &FigApp) -> bool {
    app.ui.right_tab == crate::RightTab::Design
        && app.ui.picker.is_none()
        && app.session.tool.tool == figcraft_engine::ToolId::Move
        && !app.session.tool.busy()
        && app.session.ds().is_ok_and(|d| d.text_edit.is_none() && d.vector_edit.is_none())
}

pub fn cancel(app: &mut FigApp, ctx: &egui::Context) -> bool {
    let drag = ctx.data(|d| d.get_temp::<Drag>(drag_id()));
    if let Some(drag) = &drag {
        figcraft_engine::selection_spacing::cancel_gesture(&mut app.session, drag.uid);
        ctx.data_mut(|d| d.remove::<Drag>(drag_id()));
    }
    drag.is_some()
}

/// Retire handles when another document, page, tool or editing context takes over.
pub fn sync(app: &mut FigApp, ctx: &egui::Context) {
    if let Some(drag) = ctx.data(|d| d.get_temp::<Drag>(drag_id())) {
        let same = app.session.ds().is_ok_and(|d| d.uid == drag.uid && d.page == drag.page && d.selection == drag.ids && d.interaction.is_some());
        if !same || !available(app) {
            cancel(app, ctx);
        }
    }
}

/// Called before canvas pointer dispatch, also when the pointer leaves the canvas/window.
pub fn input(app: &mut FigApp, ctx: &egui::Context, pos: Option<Pos2>, hovered: bool, space: bool) -> bool {
    let dragging = ctx.data(|d| d.get_temp::<Drag>(drag_id()));
    if let Some(mut drag) = dragging {
        let same = app.session.ds().is_ok_and(|d| d.uid == drag.uid && d.page == drag.page && d.selection == drag.ids && d.interaction.is_some());
        if !same {
            cancel(app, ctx);
            return true;
        }
        let down = ctx.input(|i| i.pointer.primary_down());
        if let Some(p) = pos {
            match &mut drag.action {
                Action::Spacing { axis, value: original, last_value } => {
                    let zoom = app.session.ds().map_or(1.0, |d| d.view.zoom).max(1e-6);
                    let distance = if axis.vertical { p.y - drag.start.y } else { p.x - drag.start.x };
                    drag.moved |= distance.abs() >= 3.0;
                    if drag.moved {
                        let value = ctx
                            .input(|i| {
                                let quantum = if i.modifiers.alt {
                                    0.1
                                } else if i.modifiers.shift {
                                    10.0
                                } else {
                                    1.0
                                };
                                ((*original + f64::from(distance) / zoom) / quantum).round() * quantum
                            })
                            .clamp(-1e6, 1e6);
                        let key = if axis.vertical { "vertical" } else { "horizontal" };
                        if *last_value != Some(value) {
                            match app.session.execute("arrange.spacing", &json!({"ids":drag.ids,key:value,"fromGesture":true})) {
                                Ok(_) => *last_value = Some(value),
                                Err(e) => app.toast(e.to_string()),
                            }
                        }
                    }
                }
                Action::Reorder { layout, source, target, swap } if app.canvas.rect.contains(p) => {
                    drag.moved |= p.distance(drag.start) >= 3.0;
                    // Hit original slots, never the moving preview: otherwise a stationary
                    // pointer could alternate between two permutations on successive frames.
                    let nearest = layout
                        .slots
                        .iter()
                        .min_by(|a, b| {
                            let center = |b: figcraft_geom::Rect| app.to_screen((b.x0 + b.x1) * 0.5, (b.y0 + b.y1) * 0.5);
                            center(a.1).distance_sq(p).total_cmp(&center(b.1).distance_sq(p))
                        })
                        .map(|(id, _)| *id)
                        .unwrap_or(*source);
                    let swapping = ctx.input(|i| i.modifiers.command);
                    if drag.moved && (nearest != *target || swapping != *swap) {
                        match app
                            .session
                            .execute("arrange.reorder", &json!({"ids":drag.ids,"source":source,"target":nearest,"swap":swapping,"fromGesture":true}))
                        {
                            Ok(_) => {
                                *target = nearest;
                                *swap = swapping;
                            }
                            Err(e) => app.toast(e.to_string()),
                        }
                    }
                }
                // A release over a panel or outside the window commits the last canvas
                // preview; the release's sentinel coordinates must not select a new slot.
                Action::Reorder { .. } => {}
            }
        }
        ctx.set_cursor_icon(match &drag.action {
            Action::Spacing { axis, .. } if axis.vertical => CursorIcon::ResizeVertical,
            Action::Spacing { .. } => CursorIcon::ResizeHorizontal,
            Action::Reorder { .. } => CursorIcon::Grabbing,
        });
        if !down {
            // A plain press on a reorder ring that didn't become a drag is a click on that layer:
            // the selection narrows to it, as a click on the canvas would. The press's modifiers
            // decide (letting Shift go before the release doesn't make it a plain click), and a
            // release away from where it pressed (outside the canvas, say) ends a drag, not a click.
            let released = pos.is_some_and(|p| app.canvas.rect.contains(p) && p.distance(drag.start) < 3.0);
            let clicked = match &drag.action {
                Action::Reorder { source, .. } if drag.plain && !drag.moved && released => Some(*source),
                _ => None,
            };
            let _ = app.session.commit_interaction();
            ctx.data_mut(|d| d.remove::<Drag>(drag_id()));
            if let Some(id) = clicked {
                app.session.set_selection(vec![id]);
            }
        } else {
            ctx.data_mut(|d| d.insert_temp(drag_id(), drag));
        }
        return true;
    }
    if !hovered || space || !available(app) {
        return false;
    }
    let Some(p) = pos else { return false };
    let ids = app.session.selection();
    if let Some(reorder) = reorder_layout(app, ctx, &ids) {
        for (id, b) in &reorder.slots {
            if ring(app, *b).is_some_and(|(center, r)| center.distance(p) <= r) {
                ctx.set_cursor_icon(CursorIcon::Grab);
                if ctx.input(|i| i.pointer.primary_pressed()) {
                    let source = *id;
                    start_drag(app, ctx, ids, p, Action::Reorder { layout: reorder, source, target: source, swap: false });
                }
                return true;
            }
        }
    }
    if ctx.input(|i| i.modifiers.command) {
        return false;
    }
    let layout = layout(app, ctx, &ids);
    for axis in [layout.horizontal.as_ref(), layout.vertical.as_ref()].into_iter().flatten() {
        for h in handles(app, axis) {
            if !h.hit.contains(p) {
                continue;
            }
            ctx.set_cursor_icon(if axis.vertical { CursorIcon::ResizeVertical } else { CursorIcon::ResizeHorizontal });
            if ctx.input(|i| i.pointer.primary_pressed()) {
                start_drag(app, ctx, ids, p, Action::Spacing { axis: axis.clone(), value: h.value, last_value: axis.value });
            }
            return true;
        }
    }
    false
}

pub fn draw(app: &FigApp, painter: &egui::Painter) {
    let ctx = painter.ctx();
    let drag = ctx.data(|d| d.get_temp::<Drag>(drag_id()));
    if !available(app) && drag.is_none() {
        return;
    }
    let Ok(ds) = app.session.ds() else { return };
    let hover = ctx.input(|i| i.pointer.hover_pos());
    let Some(b) = bounds(&ds.doc, &ds.selection) else { return };
    let rect = Rect::from_two_pos(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1)).expand(12.0);
    if drag.is_none() && !hover.is_some_and(|p| rect.contains(p)) {
        return;
    }
    let reorder = match drag.as_ref().map(|d| &d.action) {
        Some(Action::Reorder { layout, .. }) => Some(layout.clone()),
        Some(Action::Spacing { .. }) => None,
        None => reorder_layout(app, ctx, &ds.selection),
    };
    if let Some(layout) = reorder {
        for (id, _) in &layout.slots {
            let Some((center, r)) = ds.doc.world_bounds(*id).and_then(|b| ring(app, b)) else { continue };
            let hot = hover.is_some_and(|p| center.distance(p) <= r);
            let active = matches!(drag.as_ref().map(|d| &d.action), Some(Action::Reorder { source, .. }) if source == id);
            if active {
                painter.circle_filled(center, 4.5, PINK);
            } else {
                painter.circle_stroke(center, 4.5, Stroke::new(if hot { 2.5 } else { 1.5 }, PINK));
            }
        }
    }
    if let Some(Drag { action: Action::Reorder { layout, target, .. }, moved: true, .. }) = &drag {
        if let Some((_, b)) = layout.slots.iter().find(|(id, _)| id == target) {
            let rect = Rect::from_two_pos(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1)).expand(3.0);
            painter.rect_stroke(rect, 0, Stroke::new(2.0, crate::theme::ACCENT), egui::StrokeKind::Outside);
        }
        return;
    }
    let axes = if let Some(Drag { action: Action::Spacing { axis, .. }, .. }) = &drag {
        vec![axis.clone()]
    } else {
        let layout = layout(app, ctx, &ds.selection);
        [layout.horizontal.clone(), layout.vertical.clone()].into_iter().flatten().collect()
    };
    for axis in axes {
        for h in handles(app, &axis) {
            let hot = hover.is_some_and(|p| h.hit.contains(p)) || drag.is_some();
            let by = if axis.vertical { vec2(10.0, 0.0) } else { vec2(0.0, 10.0) };
            painter.line_segment([h.center - by, h.center + by], Stroke::new(if hot { 3.0 } else { 2.0 }, PINK));
            if hot {
                let at = h.center + if axis.vertical { vec2(18.0, 0.0) } else { vec2(0.0, -22.0) };
                pink_label(painter, widgets::fmt_num(h.value), at);
            }
        }
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use crate::NoServices;
    use figcraft_engine::Session;

    fn setup() -> (FigApp, egui::Context, Vec<NodeId>) {
        let mut session = Session::new();
        let mut ids = Vec::new();
        for x in [0, 80, 160] {
            ids.push(NodeId(session.execute("shape.rectangle", &json!({"x":x,"width":50,"height":50})).unwrap()["id"].as_u64().unwrap()));
        }
        session.execute("select.set", &json!({"ids":ids})).unwrap();
        (FigApp::new(session, Box::new(NoServices)), egui::Context::default(), ids)
    }

    #[test]
    fn layout_is_shared_within_a_frame_and_refreshed_after_a_document_edit() {
        let (mut app, ctx, ids) = setup();
        let mut cached = None;
        let mut output = ctx.run_ui(Default::default(), |_| {
            let first = layout(&app, &ctx, &ids);
            assert_eq!(first.horizontal.as_ref().unwrap().value, Some(30.0));
            assert!(Arc::ptr_eq(&first, &layout(&app, &ctx, &ids)));
            cached = Some(first);
        });
        output.textures_delta.clear();
        let mut output = ctx.run_ui(Default::default(), |_| {
            let first = layout(&app, &ctx, &ids);
            assert!(Arc::ptr_eq(cached.as_ref().unwrap(), &first), "unchanged frames reuse the calculation");
            app.set_props(&[ids[1]], json!({"x":90}));
            assert!(Arc::ptr_eq(&first, &layout(&app, &ctx, &ids)), "a mid-frame edit waits until the next frame");
        });
        output.textures_delta.clear();
        let mut output = ctx.run_ui(Default::default(), |_| {
            let next = layout(&app, &ctx, &ids);
            assert!(!Arc::ptr_eq(cached.as_ref().unwrap(), &next));
            assert_eq!(next.horizontal.as_ref().unwrap().value, None);
            assert!(Arc::ptr_eq(&next, &layout(&app, &ctx, &ids)));
        });
        output.textures_delta.clear();
    }

    #[test]
    fn a_selection_or_document_switch_invalidates_the_layout() {
        let (mut app, ctx, ids) = setup();
        let mut output = ctx.run_ui(Default::default(), |_| {
            let all = layout(&app, &ctx, &ids);
            let subset = layout(&app, &ctx, &ids[..2]);
            assert!(!Arc::ptr_eq(&all, &subset));
            assert_eq!(subset.horizontal.as_ref().unwrap().tracks.len(), 2);
            app.session.execute("file.new", &json!({})).unwrap();
            let other = layout(&app, &ctx, &ids[..2]);
            assert!(!Arc::ptr_eq(&subset, &other));
            assert!(other.horizontal.is_none() && other.vertical.is_none());
        });
        output.textures_delta.clear();
    }
}
