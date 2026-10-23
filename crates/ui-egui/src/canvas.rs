//! The canvas: the rendered page plus selection, hover, guides, labels and text-editing overlays.

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_engine::Preedit;
use figcraft_engine::tools::{PointerEvent, PointerKind, ToolId, selection_box, selection_frame};
use figcraft_geom::Point;
use serde_json::json;

use crate::FigApp;
use crate::i18n::{tr, trf, trn};
use crate::render_worker::{Done, Job, Worker};
use crate::theme::{COMPONENT, Tokens, doc_c32, regular, semibold};

pub struct CanvasState {
    pub rect: Rect,
    texture: Option<egui::TextureHandle>,
    tex_view: Option<figcraft_render::View>,
    /// The file ([`figcraft_engine::DocState::uid`]) and page the texture shows: it is drawn only
    /// on them.
    tex_of: Option<(u64, NodeId)>,
    /// The texture is a coarse preview (see [`crate::render_worker`]).
    pub tex_coarse: bool,
    submitted: Option<JobKey>,
    /// The view of the last render asked for.
    want_view: Option<figcraft_render::View>,
    /// What the last canvas render had to do (shown by `ui.inspect`).
    pub render_work: Option<figcraft_render::Work>,
    /// From asking for a render to its preview, and its refined image, handed to egui as the
    /// canvas texture (ms; the texture is uploaded when that frame is painted).
    pub preview_latency_ms: Option<f64>,
    pub refine_latency_ms: Option<f64>,
    dragging: bool,
    space_pan: bool,
    middle_pan: Option<(Pos2, f64, f64)>,
    /// The zoom tool (Z held): zooming out (Alt) or in, and where its drag started.
    zoom_tool: Option<bool>,
    zoom_drag: Option<Pos2>,
    pub last_pointer: Option<Point>,
    /// The real pointer was over the canvas last frame (leaving it clears what it hovered).
    hovering: bool,
    /// What the pointer shows over the canvas (`ui.inspect`); none while it is elsewhere or
    /// something on the canvas with a cursor of its own has it (a ruler guide, a gradient handle).
    pub cursor: Option<crate::cursors::Shown>,
    preedit: Option<PreeditView>,
    /// The pen pressure last reported during the current press (pen and stylus touches; the
    /// winit backends and browsers that report it).
    pressure: Option<f32>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            rect: Rect::NOTHING,
            texture: None,
            tex_view: None,
            tex_of: None,
            tex_coarse: false,
            submitted: None,
            want_view: None,
            render_work: None,
            preview_latency_ms: None,
            refine_latency_ms: None,
            dragging: false,
            space_pan: false,
            middle_pan: None,
            cursor: None,
            zoom_tool: None,
            zoom_drag: None,
            last_pointer: None,
            hovering: false,
            preedit: None,
            pressure: None,
        }
    }
}

/// What the canvas shows while an IME composition is in progress: the document with the
/// composition inserted at the caret (the real document only gets it on commit).
pub struct PreeditView {
    /// The document, composition and caret it was built from.
    key: (Arc<Document>, Preedit, usize),
    pub doc: Arc<Document>,
    /// The composition's char range, and the IME cursor, in `doc`'s edited text.
    pub range: (usize, usize),
    pub cursor: usize,
}

impl FigApp {
    fn preedit_key(&self) -> Option<(Arc<Document>, Preedit, usize)> {
        let ds = self.session.ds().ok()?;
        let te = ds.text_edit.as_ref()?;
        Some((ds.doc.clone(), te.preedit.clone()?, te.caret))
    }

    /// Rebuild the composition view when the document or composition changed.
    pub fn sync_preedit(&mut self) {
        if self.preedit_view().is_some() {
            return;
        }
        self.canvas.preedit = self.preedit_key().and_then(|key| {
            let (doc, range, cursor) = figcraft_engine::textedit::preedit_doc(&self.session)?;
            Some(PreeditView { key, doc: Arc::new(doc), range, cursor })
        });
    }

    /// The composition view, when it matches the current state.
    pub fn preedit_view(&self) -> Option<&PreeditView> {
        let p = self.canvas.preedit.as_ref()?;
        let (doc, pe, caret) = self.preedit_key()?;
        (Arc::ptr_eq(&p.key.0, &doc) && p.key.1 == pe && p.key.2 == caret).then_some(p)
    }
}

#[derive(Clone, PartialEq)]
struct JobKey {
    doc: usize,
    page: NodeId,
    origin: (i64, i64),
    scale: i64,
    size: (u32, u32),
    outline: bool,
    outline_extras: figcraft_render::Outline,
    outlined: Vec<NodeId>,
    skip: Vec<NodeId>,
    solo: Option<NodeId>,
    /// Text lays out anew when fonts arrive (system fonts load in the background).
    fonts: u64,
    proof: bool,
}

impl FigApp {
    /// Page → screen (logical points).
    pub fn to_screen(&self, x: f64, y: f64) -> Pos2 {
        let Ok(ds) = self.session.ds() else { return Pos2::ZERO };
        let (sx, sy) = ds.view.page_to_screen(x, y);
        pos2(self.canvas.rect.min.x + sx as f32, self.canvas.rect.min.y + sy as f32)
    }
    pub fn to_page(&self, p: Pos2) -> Point {
        let Ok(ds) = self.session.ds() else { return Point::ZERO };
        let (x, y) = ds.view.screen_to_page(f64::from(p.x - self.canvas.rect.min.x), f64::from(p.y - self.canvas.rect.min.y));
        Point::new(x, y)
    }
}

fn pointer_event(app: &FigApp, kind: PointerKind, pos: Pos2, m: &egui::Modifiers) -> PointerEvent {
    let p = app.to_page(pos);
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
    PointerEvent {
        kind,
        x: p.x,
        y: p.y,
        sx: f64::from(pos.x),
        sy: f64::from(pos.y),
        shift: m.shift,
        alt: m.alt,
        cmd: m.command,
        ctrl: m.ctrl,
        zoom,
        pressure: app.canvas.pressure,
    }
}

pub fn show(app: &mut FigApp, ui: &mut Ui, worker: &mut Worker) {
    let rect = ui.max_rect();
    app.canvas.rect = rect;
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    let resp = ui.interact(rect, ui.id().with("canvas"), Sense::click_and_drag());
    let key_box = app.ui.key_box;
    crate::a11y::canvas(&mut app.ui.nav, &resp, || key_box.map_or_else(|| describe(&app.session), describe_box));
    if let Ok(ds) = app.session.ds_mut()
        && ((ds.view.vw - f64::from(rect.width())).abs() > 0.5 || (ds.view.vh - f64::from(rect.height())).abs() > 0.5)
    {
        // Keep the centre; the first sizing after opening re-fits.
        let first = ds.view.vw == 1200.0 && ds.view.vh == 800.0;
        ds.view.vw = f64::from(rect.width());
        ds.view.vh = f64::from(rect.height());
        if first {
            ds.zoom_to_fit();
        }
    }
    let own_cursor = input(app, ui, &resp);
    // A browser names no dragged file until the drop, so documents there still say "import".
    let documents = ctx.input(|i| i.raw.hovered_files.iter().all(|f| f.path.as_deref().is_some_and(crate::is_document)));
    crate::drop_overlay(&ctx, ctx.content_rect(), if documents { tr("Drop to open") } else { tr("Drop to import") });
    crate::guides::end_released(&ctx);
    crate::live::track_pointer(app, &ctx);
    app.sync_preedit();
    // Retained rendering snaps the view outward and adds one pixel on each axis.
    let max_texture_side = (ctx.input(|i| i.max_texture_side).min(16_384) as u32).saturating_sub(1).max(1);
    request_render(app, worker, ppp, max_texture_side);
    // Every finished render is shown, even when a newer one is already on its way: renders finish
    // in order, and an edit every frame (a drag) would otherwise never show until it stopped.
    let shown = app.session.ds().map(|ds| (ds.uid, ds.page)).ok();
    if let Some(done) = worker.poll()
        && shows(&app.canvas, &done, shown)
    {
        if !done.coarse {
            app.canvas.render_work = Some(done.work);
            app.ui.render_ms = done.ms;
        }
        // A pixel preview render is magnified with hard pixel edges.
        let filter = if app.session.ds().ok().and_then(|d| d.view.pixel_preview_scale()) == Some(done.view.scale) {
            egui::TextureOptions::NEAREST
        } else {
            egui::TextureOptions::LINEAR
        };
        match &mut app.canvas.texture {
            Some(t) => t.set(done.image, filter),
            None => app.canvas.texture = Some(ctx.load_texture("figcraft-canvas", done.image, filter)),
        }
        let latency = Some(done.at.elapsed().as_secs_f64() * 1000.0);
        if done.coarse {
            app.canvas.preview_latency_ms = latency;
        } else {
            app.canvas.refine_latency_ms = latency;
        }
        app.canvas.tex_view = Some(done.view);
        app.canvas.tex_of = Some((done.uid, done.page));
        app.canvas.tex_coarse = done.coarse;
    }
    let painter = ui.painter_at(rect);
    let t = Tokens::get(&ctx);
    // Page background (also behind a stale/partial texture).
    let bg = app
        .session
        .ds()
        .ok()
        .and_then(|ds| match ds.doc.node(ds.page).map(|n| &n.kind) {
            Some(NodeKind::Page { background, .. }) => Some(doc_c32(&ctx, *background)),
            _ => None,
        })
        .unwrap_or(t.canvas);
    painter.rect_filled(rect, CornerRadius::ZERO, bg);
    crate::theme::set_canvas_page(&ctx, bg);
    if let (Some(tex), Some(v), true) = (&app.canvas.texture, &app.canvas.tex_view, shown.is_some() && app.canvas.tex_of == shown) {
        // Re-project the last render onto the current view.
        let tl = app.to_screen(v.origin.0, v.origin.1);
        let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
        let pixel_preview = app.session.ds().ok().and_then(|ds| ds.view.pixel_preview_scale()) == Some(v.scale);
        let image_rect = presented_image_rect(tl, v, zoom, ppp, pixel_preview);
        painter.image(tex.id(), image_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    app.session.tool.labels = overlays(app, &painter, &t);
    crate::frame_labels::rename_field(app, ui);
    // The zoom tool's area.
    if let (Some(start), Some(p)) = (app.canvas.zoom_drag, ctx.input(|i| i.pointer.hover_pos())) {
        let area = Rect::from_two_pos(start, p);
        if area.width() > 4.0 || area.height() > 4.0 {
            painter.rect_filled(area, CornerRadius::ZERO, accent(&painter).gamma_multiply(0.1));
            painter.rect_stroke(area, CornerRadius::ZERO, Stroke::new(1.0, accent(&painter)), StrokeKind::Inside);
        }
    }
    crate::dev_mode::draw(app, &painter);
    crate::handles::draw(app, &painter);
    crate::selection_spacing::draw(app, &painter);
    crate::comments::draw(app, &painter);
    crate::live_text::draw(app, &painter);
    crate::live::draw(app, &painter, &ctx);
    crate::live::chat_box(app, &ctx);
    crate::live::spotlight_banner(app, &ctx, rect);
    crate::comments::popovers(app, &ctx);
    crate::versions::preview_banner(app, &ctx, rect);
    crate::branch_ui::banner(app, &ctx, rect);
    crate::dev_mode::focus_bar(app, &ctx, rect);
    // The cursor: the tool's, unless something on the canvas with its own has the pointer.
    app.canvas.cursor = None;
    if let (true, Some(at)) = (own_cursor && resp.hovered(), ctx.input(|i| i.pointer.hover_pos())) {
        let panning = app.canvas.space_pan || app.canvas.middle_pan.is_some();
        let shown = crate::cursors::on_canvas(panning, ctx.input(|i| i.pointer.any_down()), app.canvas.zoom_tool, app.session.tool.overlay.cursor);
        shown.show(&ctx, at);
        app.canvas.cursor = Some(shown);
    }
}

/// The image's screen rect; align native-density renders to device pixels to avoid linear filtering
/// blending neighbouring texels. Stale and pixel-preview renders retain their original projection.
pub(super) fn presented_image_rect(tl: Pos2, view: &figcraft_render::View, zoom: f64, ppp: f32, pixel_preview: bool) -> Rect {
    let w = (f64::from(view.width) / view.scale * zoom) as f32;
    let h = (f64::from(view.height) / view.scale * zoom) as f32;
    let native_density = !pixel_preview
        && ppp.is_finite()
        && ppp > 0.0
        && zoom.is_finite()
        && view.scale.is_finite()
        && view.scale > 0.0
        && (view.scale - zoom * f64::from(ppp)).abs() <= view.scale.abs().max(1.0) * 1e-9;
    let min = if native_density { pos2((tl.x * ppp).round() / ppp, (tl.y * ppp).round() / ppp) } else { tl };
    Rect::from_min_size(min, vec2(w, h))
}

/// What the canvas says to screen readers: the selected layer ("Rectangle 1, Rectangle, 120 by 80,
/// selected"), how many are selected, or the page with nothing selected.
pub fn describe(s: &figcraft_engine::Session) -> String {
    let Ok(ds) = s.ds() else { return String::new() };
    match ds.selection.as_slice() {
        [] => trf("{page}, nothing selected", &[("page", &ds.doc.node(ds.page).map(|p| p.name.as_str()).unwrap_or(tr("Page")))]),
        [id] => match ds.doc.node(*id) {
            Some(n) => {
                let kind = tr(n.kind.label());
                let name = if n.name.is_empty() { kind } else { n.name.as_str() };
                let (w, h) = (crate::widgets::fmt_num(n.width), crate::widgets::fmt_num(n.height));
                trf("{name}, {kind}, {w} by {h}, selected", &[("name", &name), ("kind", &kind), ("w", &w), ("h", &h)])
            }
            None => String::new(),
        },
        many => trn("{n} layers selected", many.len(), &[]),
    }
}

/// What the canvas says while a keyboard selection box shows: its size and place, and the keys.
fn describe_box([x0, y0, x1, y1]: [f64; 4]) -> String {
    let n = crate::widgets::fmt_num;
    let (w, h, x, y) = (n((x1 - x0).abs()), n((y1 - y0).abs()), n(x0.min(x1)), n(y0.min(y1)));
    let cmd = crate::toolbar::pretty_shortcut("Cmd");
    trf(
        "Selection box, {w} by {h} at {x}, {y}: arrows move it, Shift+arrows or {cmd}+arrows size it, Enter selects, Escape cancels",
        &[("w", &w), ("h", &h), ("x", &x), ("y", &y), ("cmd", &cmd)],
    )
}

/// Whether a finished render goes on screen: an image of the file and page shown (`shown`), and
/// not a coarse preview over a refined image that already shows the whole view asked for at its
/// zoom (the same zoom elsewhere on the page is no such image).
fn shows(c: &CanvasState, done: &Done, shown: Option<(u64, NodeId)>) -> bool {
    if shown != Some((done.uid, done.page)) {
        return false;
    }
    let refined_here = !c.tex_coarse
        && c.tex_of == shown
        && c.tex_view
            .as_ref()
            .zip(c.want_view.as_ref())
            .is_some_and(|(have, want)| have.scale.to_bits() == want.scale.to_bits() && covers(have, want));
    !(done.coarse && refined_here)
}

/// Whether `outer` shows all of the page that `inner` does.
fn covers(outer: &figcraft_render::View, inner: &figcraft_render::View) -> bool {
    let (o, i) = (outer.page_rect(), inner.page_rect());
    // Rounding in the snapped origin, well under a device pixel.
    let eps = 1e-6 / inner.scale.max(1e-9);
    o.x0 <= i.x0 + eps && o.y0 <= i.y0 + eps && o.x1 >= i.x1 - eps && o.y1 >= i.y1 - eps
}

fn request_render(app: &mut FigApp, worker: &mut Worker, ppp: f32, max_texture_side: u32) {
    let Ok(ds) = app.session.ds() else { return };
    let rect = app.canvas.rect;
    let (mut ox, mut oy) = ds.view.origin();
    let (scale, visible_w, visible_h) = if let Some(k) = ds.view.pixel_preview_scale() {
        // Pixel preview: render `k` pixels per page unit from a whole-pixel origin, shown magnified
        // without smoothing.
        (ox, oy) = ((ox * k).floor() / k, (oy * k).floor() / k);
        let z = ds.view.zoom.max(1e-9) / k;
        (k, (f64::from(rect.width()) / z).ceil() as u32 + 2, (f64::from(rect.height()) / z).ceil() as u32 + 2)
    } else {
        (ds.view.zoom * f64::from(ppp), (rect.width() * ppp).ceil().max(1.0) as u32, (rect.height() * ppp).ceil().max(1.0) as u32)
    };
    // The edited text is drawn live (an IME composition included); the caret is an overlay.
    let doc = app.preedit_view().map(|p| p.doc.clone()).unwrap_or_else(|| ds.doc.clone());
    let outlined = if ds.view.outlines_selection { ds.selection.clone() } else { vec![] };
    let proof = ds.view.proof_srgb && !doc.color_profile.is_srgb();
    let outline_extras = figcraft_render::Outline { hidden: ds.view.outline_hidden, bounds: ds.view.outline_bounds, light: false };
    // Render beyond the visible viewport. During a pan the previous texture then still covers
    // the screen while the worker renders newly exposed content. Request a new buffer before the
    // visible area reaches its edge, rather than exposing an empty strip each pointer move.
    let margin = overscan_margin(visible_w, visible_h, max_texture_side);
    let origin = (ox, oy);
    let visible = (visible_w, visible_h);
    let requested = (ox - f64::from(margin) / scale, oy - f64::from(margin) / scale);
    let size = (visible_w.saturating_add(margin * 2).min(max_texture_side), visible_h.saturating_add(margin * 2).min(max_texture_side));
    let key = JobKey {
        doc: Arc::as_ptr(&doc) as usize,
        page: ds.page,
        origin: ((requested.0 * 1000.0) as i64, (requested.1 * 1000.0) as i64),
        scale: (scale * 10000.0) as i64,
        size,
        outline: ds.view.outlines,
        outline_extras,
        outlined: outlined.clone(),
        skip: vec![],
        solo: ds.focused(),
        fonts: figcraft_text::layout_generation(),
        proof,
    };
    if let Some(previous) = &app.canvas.submitted {
        let previous_origin = (previous.origin.0 as f64 / 1000.0, previous.origin.1 as f64 / 1000.0);
        let same_content = JobKey { origin: previous.origin, size: previous.size, ..key.clone() } == *previous;
        let covers = cached_render_covers(previous_origin, previous.size, origin, visible, scale);
        if same_content && covers {
            return;
        }
    }
    if app.canvas.submitted.as_ref() == Some(&key) {
        return;
    }
    let job = Job {
        doc,
        page: ds.page,
        view: figcraft_render::View { origin: requested, scale, width: size.0, height: size.1 },
        outline: ds.view.outlines,
        outline_extras,
        outlined,
        skip: vec![],
        solo: key.solo,
        proof,
        uid: ds.uid,
        zoom_preview: ds.view.pixel_preview_scale().is_none(),
        at: web_time::Instant::now(),
    };
    app.canvas.want_view = Some(job.view.clone());
    worker.submit(job);
    app.canvas.submitted = Some(key);
}

/// Leave a 96-pixel reserve so the next render starts before a pan reaches a cached edge.
fn cached_render_covers(buffer_origin: (f64, f64), buffer_size: (u32, u32), view_origin: (f64, f64), view_size: (u32, u32), scale: f64) -> bool {
    if !scale.is_finite() || scale <= 0.0 {
        return false;
    }
    let guard_x = 96.0_f64.min(f64::from(buffer_size.0.saturating_sub(view_size.0)) / 4.0) / scale;
    let guard_y = 96.0_f64.min(f64::from(buffer_size.1.saturating_sub(view_size.1)) / 4.0) / scale;
    view_origin.0 >= buffer_origin.0 + guard_x
        && view_origin.1 >= buffer_origin.1 + guard_y
        && view_origin.0 + f64::from(view_size.0) / scale <= buffer_origin.0 + f64::from(buffer_size.0) / scale - guard_x
        && view_origin.1 + f64::from(view_size.1) / scale <= buffer_origin.1 + f64::from(buffer_size.1) / scale - guard_y
}

/// Bound the extra rendered area to about 1.6 times the visible area. A fixed large margin can
/// multiply the pixel count several-fold on a narrow or small viewport.
fn overscan_margin(width: u32, height: u32, max_texture_side: u32) -> u32 {
    width.min(height).saturating_div(8).min(128).min(max_texture_side.saturating_sub(width.max(height)) / 2)
}

#[cfg(test)]
#[test]
fn pan_uses_cached_pixels_until_the_refill_boundary() {
    let margin = overscan_margin(1000, 800, 16_383);
    assert_eq!(margin, 100);
    let buffer = (-f64::from(margin), -f64::from(margin));
    let size = (1000 + 2 * margin, 800 + 2 * margin);
    assert!(cached_render_covers(buffer, size, (0.0, 0.0), (1000, 800), 1.0));
    assert!(cached_render_covers(buffer, size, (50.0, 0.0), (1000, 800), 1.0));
    assert!(!cached_render_covers(buffer, size, (80.0, 0.0), (1000, 800), 1.0));
    assert!(!cached_render_covers(buffer, size, (-80.0, 0.0), (1000, 800), 1.0));
    assert!(cached_render_covers((-24.0, -24.0), (1048, 848), (10.0, 0.0), (1000, 800), 1.0));
    assert!(!cached_render_covers((-24.0, -24.0), (1048, 848), (19.0, 0.0), (1000, 800), 1.0));
}

#[cfg(test)]
#[test]
fn overscan_stays_within_texture_and_pixel_budget() {
    for (width, height, limit) in [(1000, 800, 16_383), (390, 844, 16_383), (2560, 1440, 16_383), (1000, 800, 1060)] {
        let margin = overscan_margin(width, height, limit);
        assert!(width + 2 * margin <= limit || width > limit);
        assert!(height + 2 * margin <= limit || height > limit);
        let rendered = u64::from(width + 2 * margin) * u64::from(height + 2 * margin);
        let visible = u64::from(width) * u64::from(height);
        assert!(rendered * 5 <= visible * 8); // At most 1.6× the visible pixels.
    }
}

/// The zoom tool while Z is held (Alt: zoom out): a click zooms a step about the pointer, a drag
/// zooms to the dragged area. Takes the pointer from the other tools while it's active.
fn zoom_tool(app: &mut FigApp, ui: &Ui, resp: &egui::Response, pos: Pos2, mods: egui::Modifiers, typing: bool) -> bool {
    let held = !typing && !mods.command && ui.input(|i| i.key_down(egui::Key::Z));
    app.canvas.zoom_tool = held.then_some(mods.alt);
    let Some(start) = app.canvas.zoom_drag.or_else(|| (held && resp.hovered() && ui.input(|i| i.pointer.primary_pressed())).then_some(pos)) else {
        return held;
    };
    app.canvas.zoom_drag = Some(start);
    let area = Rect::from_two_pos(start, pos);
    if ui.input(|i| i.pointer.primary_down()) {
        return true;
    }
    app.canvas.zoom_drag = None;
    let (a, b) = (app.to_page(area.min), app.to_page(area.max));
    let params = if area.width() > 4.0 && area.height() > 4.0 {
        json!({"x": a.x, "y": a.y, "width": b.x - a.x, "height": b.y - a.y})
    } else {
        let p = app.to_page(pos);
        json!({"x": p.x, "y": p.y, "out": mods.alt})
    };
    app.run("view.zoomAt", params);
    true
}

/// The pointer and keys on the canvas. False when something on it with a cursor of its own has
/// the pointer (a comment pin, a ruler guide, a gradient or padding handle): the tool's cursor
/// then doesn't show.
fn input(app: &mut FigApp, ui: &mut Ui, resp: &egui::Response) -> bool {
    if crate::proto_canvas::input(app, ui, resp) {
        // A prototype gesture is a press on the canvas too: the Pages list gives up the keyboard.
        app.ui.pages_focus = None;
        return true;
    }
    let ctx = ui.ctx().clone();
    let (mods, scroll, zoom_delta, hover_pos, space) =
        ctx.input(|i| (i.modifiers, i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos(), i.key_down(egui::Key::Space)));
    let editing_text = app.session.ds().map(|d| d.text_edit.is_some()).unwrap_or(false);
    let typing_elsewhere = crate::a11y::field_focused(&ctx);
    app.canvas.space_pan = space && !editing_text && !typing_elsewhere;
    if !app.canvas.dragging && crate::selection_spacing::input(app, &ctx, hover_pos, resp.hovered(), app.canvas.space_pan) {
        return false;
    }
    // Dropped files import onto the canvas, behind a card while they do ([`crate::pending_open`]);
    // a dropped document opens as a file of its own, as on the home screen.
    let dropped = ctx.input(|i| i.raw.dropped_files.clone());
    for f in dropped {
        let path = f.path().to_path_buf();
        if crate::is_document(&path) {
            crate::pending_open::open_dropped(app, &f);
            continue;
        }
        let pt = hover_pos.map(|h| app.to_page(h)).unwrap_or_default();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped.png".into());
        let (params, read) = if path.exists() {
            (json!({"path": path.to_string_lossy(), "x": pt.x, "y": pt.y}), None)
        } else {
            (json!({"name": name, "x": pt.x, "y": pt.y}), Some(crate::dropped_bytes(&f, &name)))
        };
        let Some(target) =
            app.session.ds().ok().map(|doc| crate::pending_open::ImportTarget { uid: doc.uid, page: doc.page, name: doc.doc.name.clone() })
        else {
            continue;
        };
        app.ui.pending_opens.push(crate::pending_open::PendingOpen { label: name, params, command: "file.import", target: Some(target), read });
    }
    let Some(pos) = hover_pos.filter(|p| app.canvas.rect.contains(*p)) else {
        // A drag released off the canvas, or off the window, ends where the pointer was last seen.
        if app.canvas.dragging && !ctx.input(|i| i.pointer.primary_down()) {
            app.canvas.dragging = false;
            if let Some(at) = hover_pos.or_else(|| app.canvas.last_pointer.map(|p| app.to_screen(p.x, p.y))) {
                send(app, PointerKind::Up, at, &mods);
            }
        }
        // Leaving the canvas: nothing is hovered there any more.
        if !app.canvas.dragging && std::mem::take(&mut app.canvas.hovering) {
            app.session.vector_pointer_left();
        }
        // A pan whose button was released off the canvas is over.
        if !ctx.input(|i| i.pointer.button_down(egui::PointerButton::Middle) || i.pointer.primary_down()) {
            app.canvas.middle_pan = None;
        }
        return true;
    };
    // A press on the canvas gives the keyboard back to it (arrows nudge again).
    if resp.hovered() && ctx.input(|i| i.pointer.any_pressed()) {
        app.ui.layers_focus = false;
        app.ui.pages_focus = None;
        app.ui.key_box = None;
    }
    // Zoom (pinch, Cmd/Ctrl+scroll) about the pointer; plain scroll pans.
    if resp.hovered() {
        if (zoom_delta - 1.0).abs() > 1e-4 {
            let anchor = app.to_page(pos);
            if let Ok(ds) = app.session.ds_mut() {
                let z = ds.view.zoom * f64::from(zoom_delta);
                ds.view.set_zoom_at(z, (anchor.x, anchor.y));
                ds.revision += 1;
            }
        } else if scroll != egui::Vec2::ZERO
            && let Ok(ds) = app.session.ds_mut()
        {
            ds.view.scroll(f64::from(scroll.x), f64::from(scroll.y), mods.shift);
            ds.revision += 1;
        }
    }
    if zoom_tool(app, ui, resp, pos, mods, editing_text || typing_elsewhere) {
        return true;
    }
    // Middle-button / space panning.
    let middle_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Middle));
    if (middle_down || (app.canvas.space_pan && ctx.input(|i| i.pointer.primary_down()))) && resp.hovered() || app.canvas.middle_pan.is_some() {
        if let Ok(ds) = app.session.ds_mut() {
            match app.canvas.middle_pan {
                None => app.canvas.middle_pan = Some((pos, ds.view.cx, ds.view.cy)),
                Some((p0, cx, cy)) => {
                    let z = ds.view.zoom.max(1e-6);
                    ds.view.cx = cx - f64::from(pos.x - p0.x) / z;
                    ds.view.cy = cy - f64::from(pos.y - p0.y) / z;
                    ds.revision += 1;
                }
            }
        }
        if !middle_down && !ctx.input(|i| i.pointer.primary_down()) {
            app.canvas.middle_pan = None;
        }
        return true;
    }
    if !app.canvas.dragging && (resp.hovered() || app.ui.comments.dragging()) && crate::comments::input(app, &ctx, pos) {
        return false;
    }
    if !app.canvas.dragging && resp.hovered() && crate::guides::input(app, &ctx, pos) {
        return false;
    }
    if !app.canvas.dragging && resp.hovered() && crate::handles::input(app, &ctx, pos) {
        return false;
    }
    if resp.secondary_clicked() {
        open_context_menu(app, pos, mods.command);
        return true;
    }
    if resp.double_clicked() {
        let ev = pointer_event(app, PointerKind::DoubleClick, pos, &mods);
        if let Err(e) = app.session.pointer(&ev) {
            app.toast(e.to_string());
        }
        app.canvas.dragging = false;
        return true;
    }
    let primary_pressed = ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
    let primary_down = ctx.input(|i| i.pointer.primary_down());
    let primary_released = ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
    let kind = if primary_pressed && resp.hovered() {
        app.canvas.dragging = true;
        app.ui.context_menu = None;
        app.ui.tool_flyout = None;
        // Clicking the canvas takes focus from panel fields.
        ctx.memory_mut(|m| {
            if let Some(f) = m.focused() {
                m.surrender_focus(f);
            }
        });
        Some(PointerKind::Down)
    } else if app.canvas.dragging && primary_released {
        app.canvas.dragging = false;
        Some(PointerKind::Up)
    } else if app.canvas.dragging && primary_down {
        Some(PointerKind::Move)
    } else if resp.hovered() {
        app.canvas.hovering = true;
        Some(PointerKind::Hover)
    } else {
        // Something over the canvas took the pointer.
        if std::mem::take(&mut app.canvas.hovering) {
            app.session.vector_pointer_left();
        }
        None
    };
    // Pen pressure comes with touch events (a pen on Windows, a stylus in a browser); a mouse,
    // macOS and Linux tablets report none and draw at the stroke weight.
    let force = ctx.input(|i| {
        i.events.iter().rev().find_map(|e| match e {
            egui::Event::Touch { force: Some(f), .. } if f.is_finite() && *f > 0.0 => Some(f.clamp(0.0, 1.0)),
            _ => None,
        })
    });
    if force.is_some() {
        app.canvas.pressure = force;
    }
    if let Some(k) = kind {
        send(app, k, pos, &mods);
    }
    true
}

/// Pointer event `kind` at screen point `pos`, to the tools.
fn send(app: &mut FigApp, kind: PointerKind, pos: Pos2, m: &egui::Modifiers) {
    let ev = pointer_event(app, kind, pos, m);
    if kind == PointerKind::Up {
        app.canvas.pressure = None;
    }
    app.canvas.last_pointer = Some(Point::new(ev.x, ev.y));
    if let Err(e) = app.session.pointer(&ev) {
        app.toast(e.to_string());
    }
    app.handle_ui_requests();
}

/// Right-click at screen `pos`: select what's under the pointer (`deep`: the nested layer), then
/// open the context menu there.
pub fn open_context_menu(app: &mut FigApp, pos: Pos2, deep: bool) {
    let p = app.to_page(pos);
    if let Ok(ds) = app.session.ds() {
        let (doc, page, sel, zoom) = (ds.doc.clone(), ds.page, ds.selection.clone(), ds.view.zoom.max(1e-6));
        if let Some(leaf) = app.session.hit(&doc, page, p, 2.0 / zoom) {
            let target = figcraft_doc::geometry::select_target(&doc, leaf, &sel, deep);
            if !sel.contains(&target) {
                app.session.set_selection(vec![target]);
            }
        }
    }
    app.ui.context_menu = Some((pos, app.session.selection()));
}

fn stroke(c: Color32, w: f32) -> Stroke {
    Stroke::new(w, c)
}

/// A text layer's layout placed on the canvas: where carets and selected characters show (yours
/// and other people's in a live session).
pub(crate) struct TextOnScreen {
    pub layout: Arc<figcraft_text::TextLayout>,
    /// How many characters the text has.
    pub chars: usize,
    /// The layout's vertical offset in the box (vertical alignment).
    dy: f64,
    world: figcraft_geom::Affine,
}

impl TextOnScreen {
    /// Text layer `id` of `doc` (`None`: not a text layer).
    pub(crate) fn of(doc: &Document, id: NodeId) -> Option<Self> {
        let n = doc.node(id)?;
        let NodeKind::Text(t) = &n.kind else { return None };
        let (layout, dy) = figcraft_text::layout_node(n)?;
        Some(Self { layout, chars: t.char_count(), dy, world: doc.world_transform(id) })
    }

    /// Layout point `(x, y)` on screen (on the path, for text on a path).
    pub(crate) fn at(&self, app: &FigApp, x: f64, y: f64) -> Pos2 {
        let q = self.layout.warp_point(x, y);
        let p = self.world * Point::new(q.x, q.y + self.dy);
        app.to_screen(p.x, p.y)
    }

    /// The highlight of characters `a..b`, one screen quad per line.
    pub(crate) fn selection(&self, app: &FigApp, a: usize, b: usize) -> Vec<[Pos2; 4]> {
        let rects = self.layout.selection_rects(a, b);
        rects.iter().map(|r| [self.at(app, r.x0, r.y0), self.at(app, r.x1, r.y0), self.at(app, r.x1, r.y1), self.at(app, r.x0, r.y1)]).collect()
    }

    /// The caret before character `index`: its top and bottom on screen (an empty layer: a line
    /// of default height at its origin).
    pub(crate) fn caret(&self, app: &FigApp, index: usize, zoom: f64) -> [Pos2; 2] {
        let (cx, li) = self.layout.caret(index);
        match self.layout.lines.get(li).or(self.layout.lines.last()) {
            Some(line) => [self.at(app, cx, line.top), self.at(app, cx, line.top + line.height)],
            None => {
                let p = self.at(app, 0.0, 0.0);
                [p, p + vec2(0.0, 14.0 * zoom as f32)]
            }
        }
    }
}

/// A node's oriented outline in screen space.
pub(crate) fn node_quad(app: &FigApp, doc: &Document, id: NodeId) -> Option<[Pos2; 4]> {
    let n = doc.node(id)?;
    let w = doc.world_transform(id);
    let corners = [Point::new(0.0, 0.0), Point::new(n.width, 0.0), Point::new(n.width, n.height), Point::new(0.0, n.height)];
    Some(corners.map(|c| {
        let p = w * c;
        app.to_screen(p.x, p.y)
    }))
}

fn quad(painter: &egui::Painter, q: [Pos2; 4], s: Stroke) {
    painter.add(Shape::closed_line(q.to_vec(), s));
}

/// The slots of the selected components and instances (and a slot being dropped onto): a dashed
/// outline and the slot's name above its top-left corner, like a frame label (never over the
/// content).
fn slot_badges(app: &FigApp, painter: &egui::Painter, doc: &Document, sel: &[NodeId], drop: Option<NodeId>) {
    use figcraft_engine::components::{owning_component, owning_instance, slot_name, slots_of};
    let mut roots: Vec<NodeId> = Vec::new();
    for id in sel {
        let root = match doc.node(*id).map(|n| &n.kind) {
            Some(NodeKind::Instance { .. } | NodeKind::Component { .. } | NodeKind::ComponentSet { .. }) => Some(*id),
            _ => owning_instance(doc, *id).or_else(|| owning_component(doc, *id)),
        };
        if let Some(r) = root.filter(|r| !roots.contains(r)) {
            roots.push(r);
        }
    }
    let mut slots: Vec<(NodeId, String)> = roots.iter().flat_map(|r| slots_of(doc, *r)).collect();
    if let Some(d) = drop
        && !slots.iter().any(|(s, _)| *s == d)
        && let Some(name) = slot_name(doc, d)
    {
        slots.push((d, name));
    }
    for (id, name) in slots {
        let Some(q) = node_quad(app, doc, id) else { continue };
        if !Rect::from_points(&q).intersects(app.canvas.rect) {
            continue;
        }
        painter.extend(Shape::dashed_line(&[q[0], q[1], q[2], q[3], q[0]], stroke(COMPONENT, 1.0), 4.0, 3.0));
        // The label, when the slot is wide enough on screen to hold it.
        let galley = painter.layout_no_wrap(name, regular(10.0), COMPONENT);
        let (w, h) = (galley.size().x + 14.0, galley.size().y.max(11.0));
        if (q[1] - q[0]).length() < w {
            continue;
        }
        let at = q[0] - vec2(0.0, h + 2.0);
        crate::icons::paint(painter, Rect::from_min_size(pos2(at.x, at.y + (h - 11.0) / 2.0), vec2(11.0, 11.0)), crate::icons::Icon::Slot, COMPONENT);
        painter.galley(pos2(at.x + 14.0, at.y + (h - galley.size().y) / 2.0), galley, COMPONENT);
    }
}

/// The instance an Alt-drop of a component from the Assets panel swaps: tinted and outlined in
/// the component colour, with a "Swap instance" label above its top-left corner.
fn swap_highlight(app: &FigApp, painter: &egui::Painter, doc: &Document, id: NodeId) {
    let Some(q) = node_quad(app, doc, id) else { return };
    // White on the component colour, in both themes (like the canvas's selection labels).
    let ink = Color32::WHITE;
    painter.add(Shape::convex_polygon(q.to_vec(), COMPONENT.gamma_multiply(0.12), Stroke::NONE));
    quad(painter, q, stroke(COMPONENT, 2.0));
    let galley = painter.layout_no_wrap(tr("Swap instance").to_string(), semibold(10.0), ink);
    let size = vec2(galley.size().x + 26.0, 18.0);
    let top = Rect::from_points(&q).min;
    let pill = Rect::from_min_size(pos2(top.x, top.y - size.y - 4.0), size);
    painter.rect_filled(pill, CornerRadius::same(4), COMPONENT);
    crate::icons::paint(
        painter,
        Rect::from_center_size(pos2(pill.min.x + 11.0, pill.center().y), vec2(10.0, 10.0)),
        crate::icons::Icon::Instance,
        ink,
    );
    painter.galley(pos2(pill.min.x + 20.0, pill.center().y - galley.size().y / 2.0), galley, ink);
}

/// The outline colour for a layer: purple for components, sets and instances and every layer in
/// them, the page's overlay `green` otherwise.
fn outline_colour(doc: &Document, id: NodeId, green: Color32) -> Color32 {
    use figcraft_engine::components::{owning_component, owning_instance};
    let set = matches!(doc.node(id).map(|n| &n.kind), Some(NodeKind::ComponentSet { .. }));
    if set || owning_component(doc, id).is_some() || owning_instance(doc, id).is_some() { COMPONENT } else { green }
}

/// The outline path of shapes (ellipses, vectors) for hover/selection outlines.
fn outline_shape(app: &FigApp, doc: &Document, id: NodeId) -> Option<Vec<Vec<Pos2>>> {
    let n = doc.node(id)?;
    if !matches!(
        n.kind,
        NodeKind::Ellipse { .. } | NodeKind::Vector { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. } | NodeKind::BooleanOp { .. }
    ) {
        return None;
    }
    let (path, _) = figcraft_render::node_outline(doc, n)?;
    let w = doc.world_transform(id);
    let mut out = Vec::new();
    let mut cur: Vec<Pos2> = Vec::new();
    figcraft_geom::kurbo::flatten(path.iter(), 0.5 / app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0), |el| {
        use figcraft_geom::kurbo::PathEl::*;
        match el {
            MoveTo(p) => {
                if cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
                let q = w * p;
                cur.push(app.to_screen(q.x, q.y));
            }
            LineTo(p) => {
                let q = w * p;
                cur.push(app.to_screen(q.x, q.y));
            }
            ClosePath => {
                if let Some(f) = cur.first().copied() {
                    cur.push(f);
                }
            }
            _ => {}
        }
    });
    if cur.len() > 1 {
        out.push(cur);
    }
    Some(out)
}

/// A layer's outline on the canvas: its shape for ellipses, vectors, polygons, stars and boolean
/// groups, its box otherwise.
fn draw_outline(app: &FigApp, painter: &egui::Painter, doc: &Document, id: NodeId, s: Stroke) {
    match outline_shape(app, doc, id) {
        Some(lines) => {
            for l in lines {
                painter.add(Shape::line(l, s));
            }
        }
        None => {
            if let Some(q) = node_quad(app, doc, id) {
                quad(painter, q, s);
            }
        }
    }
}

/// Everything drawn over the rendered page; returns the frame labels drawn (frame, corners on the
/// page) for the engine to hit-test.
fn overlays(app: &FigApp, painter: &egui::Painter, t: &Tokens) -> Vec<(NodeId, [Point; 4])> {
    let mut labels = Vec::new();
    let Ok(ds) = app.session.ds() else { return labels };
    let doc = ds.doc.clone();
    let page = ds.page;
    let sel = ds.selection.clone();
    let zoom = ds.view.zoom;
    let view = ds.view.clone();
    // Pixel grid at high zoom.
    if view.pixel_grid_shown() {
        let r = app.canvas.rect;
        let p0 = app.to_page(r.min);
        let p1 = app.to_page(r.max);
        let col = Color32::from_black_alpha(18);
        let mut x = p0.x.floor();
        while x <= p1.x {
            let s = app.to_screen(x, 0.0).x;
            painter.line_segment([pos2(s, r.min.y), pos2(s, r.max.y)], stroke(col, 1.0));
            x += 1.0;
        }
        let mut y = p0.y.floor();
        while y <= p1.y {
            let s = app.to_screen(0.0, y).y;
            painter.line_segment([pos2(r.min.x, s), pos2(r.max.x, s)], stroke(col, 1.0));
            y += 1.0;
        }
    }
    // Layout grids and frame labels for top-level frames (also those in sections); the focus view
    // labels only its frame.
    let focus = ds.focused();
    for c in boards(&doc, page).iter().filter(|c| focus.is_none_or(|f| f == **c)) {
        let Some(n) = doc.node(*c) else { continue };
        let Some(b) = doc.world_bounds(*c) else { continue };
        // Off-screen frames draw neither grids nor a label (labels sit just outside the frame).
        let on_screen = Rect::from_two_pos(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1)).expand(20.0);
        if !on_screen.intersects(app.canvas.rect) {
            continue;
        }
        if view.layout_grids && !n.grids.is_empty() && figcraft_layout::grids_shown(&doc, *c) {
            let w = doc.world_transform(*c);
            for g in n.grids.iter().filter(|g| g.visible) {
                let col = doc_c32(painter.ctx(), g.color);
                for r in figcraft_layout::grid_lines(g, n.width, n.height) {
                    let a = w * Point::new(r.x0, r.y0);
                    let b = w * Point::new(r.x1, r.y1);
                    let (sa, sb) = (app.to_screen(a.x, a.y), app.to_screen(b.x, b.y));
                    if g.kind == figcraft_doc::GridKind::Grid {
                        painter.line_segment([sa, sb], stroke(col, 1.0));
                    } else {
                        painter.rect_filled(Rect::from_two_pos(sa, sb), CornerRadius::ZERO, col);
                    }
                }
            }
        }
        // The focus view's bar names its frame.
        let labelled = focus.is_none() && (n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. }));
        if labelled && let Some(q) = crate::frame_labels::draw(app, painter, t, &doc, *c, sel.contains(c), ds.hover == Some(*c)) {
            labels.push((*c, q.map(|p| app.to_page(p))));
        }
    }
    // Prototype connections.
    crate::proto_canvas::draw(app, painter);
    // Hover outline (measuring outlines the measured box in red instead).
    if let Some(h) = ds.hover
        && !sel.contains(&h)
        && matches!(app.session.tool.tool, ToolId::Move | ToolId::Scale)
        && app.session.prefs.highlight_on_hover
        && app.session.tool.overlay.measure_box.is_none()
    {
        draw_outline(app, painter, &doc, h, stroke(outline_colour(&doc, h, accent(painter)), 1.5));
    }
    // Inside a boolean group (an operand selected or in vector edit mode): every operand's outline,
    // so the shapes making the live result stay visible while one of them changes.
    let operands = figcraft_engine::booleans::outlined_operands(&doc, &sel, ds.vector_edit.as_ref().map(|ve| ve.id));
    for id in operands.iter().filter(|o| !sel.contains(o)) {
        draw_outline(app, painter, &doc, *id, stroke(outline_colour(&doc, *id, accent(painter)).gamma_multiply(0.6), 1.0));
    }
    // Mask outlines (a mask doesn't draw itself), frame outlines and slices, as the View menu shows.
    if view.mask_outlines || view.frame_outlines || view.show_slices {
        for id in doc.subtree(page) {
            let Some(n) = doc.node(id).filter(|n| n.visible) else { continue };
            if view.mask_outlines && n.is_mask {
                draw_outline(app, painter, &doc, id, stroke(accent(painter), 1.0));
            } else if view.frame_outlines && n.kind.is_frame_like() {
                draw_outline(app, painter, &doc, id, stroke(t.text_secondary, 1.0));
            } else if view.show_slices
                && matches!(n.kind, NodeKind::Slice)
                && let Some(q) = node_quad(app, &doc, id)
            {
                painter.extend(Shape::dashed_line(&[q[0], q[1], q[2], q[3], q[0]], stroke(t.text_secondary, 1.0), 4.0, 3.0));
            }
        }
    }
    // Drop target / insertion indicator.
    let ov = &app.session.tool.overlay;
    if let Some(d) = ov.drop_target
        && let Some(q) = node_quad(app, &doc, d)
    {
        quad(painter, q, stroke(if figcraft_engine::components::is_slot(&doc, d) { COMPONENT } else { accent(painter) }, 1.0));
    }
    slot_badges(app, painter, &doc, &sel, ov.drop_target);
    if let Some(t) = ov.swap_target {
        swap_highlight(app, painter, &doc, t);
    }
    if let Some([x0, y0, x1, y1]) = ov.insert_line {
        painter.line_segment([app.to_screen(x0, y0), app.to_screen(x1, y1)], stroke(accent(painter), 2.0));
    }
    if let Some([x0, y0, x1, y1]) = ov.drop_cell {
        let r = Rect::from_two_pos(app.to_screen(x0, y0), app.to_screen(x1, y1));
        painter.rect_filled(r, CornerRadius::same(2), accent(painter).gamma_multiply(0.12));
        let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
        painter.add(Shape::dashed_line(&pts, stroke(accent(painter), 1.5), 4.0, 3.0));
    }
    // Vector edit mode, with the transform box around several selected points.
    // The selection's size label, which measurement labels keep clear of.
    let mut size_label = None;
    if let Some(ve) = ds.vector_edit.clone()
        && let Some(o) = figcraft_engine::vectoredit::overlay(&doc, &ve)
    {
        // The Variable width tool shows width points in place of the path's points.
        let width_tool = ve.tool == figcraft_engine::vectoredit::VTool::Width;
        // Other layers edited at once go under the active one; the hover shows on its own layer.
        let ov = &app.session.tool.overlay;
        let hover_on = |id| ov.vector_hover.filter(|_| ov.vector_hover_on.unwrap_or(ve.id) == id);
        for part in &ve.others {
            if let Some(po) = figcraft_engine::vectoredit::overlay(&doc, &ve.view(part)) {
                vector_overlay(app, painter, &po, !width_tool, hover_on(part.id), false);
            }
        }
        vector_overlay(app, painter, &o, !width_tool, hover_on(ve.id), true);
        if width_tool {
            width_overlay(app, painter, &doc, &ve, zoom);
        } else if let Some(c) = app.session.vector_edit_box() {
            let label = format!("{} × {}", fmt((c[1] - c[0]).hypot()), fmt((c[3] - c[0]).hypot()));
            size_label = transform_box(app, painter, c.map(|p| app.to_screen(p.x, p.y)), accent(painter), true, 4, Some(label));
        }
    }
    // Selection.
    let editing = ds.text_edit.clone();
    if !sel.is_empty() && editing.is_none() && ds.vector_edit.is_none() {
        let col = if sel.iter().any(|i| outline_colour(&doc, *i, accent(painter)) == COMPONENT) { COMPONENT } else { accent(painter) };
        for id in &sel {
            draw_outline(app, painter, &doc, *id, stroke(col, 1.0));
        }
        // The shape builder reshapes by regions: no transform box.
        if app.session.tool.tool != ToolId::ShapeBuilder
            && let Some(b) = selection_box(&doc, &sel)
            && let Some((_, w, h)) = selection_frame(&doc, &sel)
        {
            let is_line = sel.len() == 1 && matches!(sel.first().and_then(|i| doc.node(*i)).map(|n| &n.kind), Some(NodeKind::Line));
            let mut label = if is_line { fmt(w) } else { format!("{} × {}", fmt(w), fmt(h)) };
            let scale = app.session.tool.overlay.scale;
            if let Some(hint) = scale {
                label = format!("{label} · {}x", fmt(hint.factor.abs()));
            }
            // A layer smaller than its handles on screen shows just its outline (they still answer
            // just outside it).
            let tiny = figcraft_engine::tools::box_sides_px(&b, zoom).1 < figcraft_engine::tools::SMALL_BOX_PX;
            // Measuring to another layer (Alt held) shows just the outlines: no handles or size.
            let measuring = app.session.tool.overlay.measure_box.is_some();
            // Locked layers (selected in the layers panel) can't be resized: no handles either.
            let handles = if measuring || sel.iter().any(|i| figcraft_engine::tools::locked_in(&doc, *i)) {
                0
            } else if is_line {
                2
            } else if tiny {
                0
            } else {
                4
            };
            size_label = transform_box(app, painter, b.map(|p| app.to_screen(p.x, p.y)), col, sel.len() > 1, handles, (!measuring).then_some(label));
            if !measuring {
                arc_overlay(app, painter, col);
                shape_overlay(app, painter, col);
            }
            if app.session.tool.tool == ToolId::Scale {
                // The point that stays put: the dragged pivot, else the anchor.
                let pivot = scale
                    .map(|h| Point::new(h.pivot[0], h.pivot[1]))
                    .or_else(|| figcraft_engine::tools::scale_anchor_point(&doc, &sel, app.session.tool.scale_anchor));
                if let Some(p) = pivot {
                    let c = app.to_screen(p.x, p.y);
                    painter.circle_filled(c, 4.0, Color32::WHITE);
                    painter.circle_stroke(c, 4.0, stroke(col, 1.5));
                    painter.circle_filled(c, 1.5, col);
                }
            }
            // The rotation origin while it shows (Alt+R): a ring with a cross through it.
            if let Some(o) = ds.origin_handle() {
                let c = app.to_screen(o.x, o.y);
                painter.circle_filled(c, 5.0, Color32::WHITE);
                painter.circle_stroke(c, 5.0, stroke(col, 1.5));
                for d in [vec2(1.0, 0.0), vec2(0.0, 1.0)] {
                    painter.line_segment([c - d * 10.0, c - d * 5.0], stroke(col, 1.5));
                    painter.line_segment([c + d * 5.0, c + d * 10.0], stroke(col, 1.5));
                }
                painter.circle_filled(c, 1.5, col);
            }
        }
    }
    if app.session.tool.tool == ToolId::ShapeBuilder
        && let Some(arr) = figcraft_engine::shapebuilder::current(&app.session)
    {
        builder_overlay(app, painter, t, &arr);
    }
    // Text editing: caret and selection, or the IME composition underlined at the caret.
    let preedit = app.preedit_view();
    let edoc: &Document = preedit.map(|p| &*p.doc).unwrap_or(&doc);
    if let Some(te) = editing
        && let Some(text) = TextOnScreen::of(edoc, te.id)
    {
        let layout = &text.layout;
        if let Some(q) = node_quad(app, edoc, te.id) {
            quad(painter, q, stroke(accent(painter), 1.0));
        }
        let (a, b) = (te.caret.min(te.anchor), te.caret.max(te.anchor));
        if a != b && preedit.is_none() {
            for q in text.selection(app, a, b) {
                painter.add(Shape::convex_polygon(q.to_vec(), accent(painter).gamma_multiply(0.3), Stroke::NONE));
            }
        }
        if let Some(p) = preedit {
            for r in layout.selection_rects(p.range.0, p.range.1) {
                // Underline just below the baseline, across the composed chars only.
                let Some(line) = layout.lines.iter().find(|l| (l.top - r.y0).abs() < 1e-6) else { continue };
                let x1 = layout
                    .glyphs
                    .get(line.glyph_start..line.glyph_end)
                    .unwrap_or(&[])
                    .iter()
                    .filter(|g| g.char_end > g.char_start && g.char_start >= p.range.0 && g.char_end <= p.range.1)
                    .map(|g| g.x + g.advance)
                    .fold(r.x0, f64::max);
                let y = line.baseline + (line.top + line.height - line.baseline) * 0.35;
                painter.line_segment([text.at(app, r.x0, y), text.at(app, x1, y)], stroke(t.text, 1.0));
            }
        }
        let caret = preedit.map(|p| p.cursor).unwrap_or(te.caret);
        let blink = (painter.ctx().input(|i| i.time) * 2.0) as i64 % 2 == 0;
        if blink || a != b {
            painter.line_segment(text.caret(app, caret, zoom), stroke(t.text, 1.0));
        }
        painter.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
    // Smart guides and measurements (their labels kept clear of the size label).
    crate::measure_overlay::draw(app, painter, t, size_label);
    // Marquee.
    if let Some([x0, y0, x1, y1]) = ov.marquee.or(app.ui.key_box) {
        let r = Rect::from_two_pos(app.to_screen(x0, y0), app.to_screen(x1, y1));
        painter.rect_filled(r, CornerRadius::ZERO, accent(painter).gamma_multiply(0.1));
        painter.rect_stroke(r, CornerRadius::ZERO, stroke(accent(painter), 1.0), StrokeKind::Inside);
    }
    // Pen path.
    if !ov.pen.is_empty() {
        let pts: Vec<Pos2> = ov.pen.iter().map(|[x, y]| app.to_screen(*x, *y)).collect();
        if app.session.tool.tool == ToolId::Pencil {
            // The stroke it will make, in its colour and weight (at least a point when zoomed out).
            let (color, weight) = figcraft_engine::tools::PENCIL_STROKE;
            painter.add(Shape::line(pts, stroke(doc_c32(painter.ctx(), color), ((weight * zoom) as f32).max(1.0))));
        } else {
            for [x0, y0, x1, y1] in &ov.pen_handles {
                bezier_handle(painter, app.to_screen(*x0, *y0), app.to_screen(*x1, *y1), 3.5, Color32::WHITE);
            }
            for p in &pts {
                painter.circle_filled(*p, 3.5, Color32::WHITE);
                painter.circle_stroke(*p, 3.5, stroke(accent(painter), 1.0));
            }
            if let Some([x0, y0, x1, y1]) = ov.rubber {
                let (a, b) = (app.to_screen(x0, y0), app.to_screen(x1, y1));
                // The segment a click would make: along the last anchor's out handle, if it has one.
                match ov.rubber_handle {
                    Some([hx, hy]) => {
                        let h = app.to_screen(hx, hy);
                        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                            [a, h, b, b],
                            false,
                            Color32::TRANSPARENT,
                            stroke(accent(painter), 1.0),
                        ));
                    }
                    None => {
                        painter.line_segment([a, b], stroke(accent(painter), 1.0));
                    }
                }
            }
        }
    }
    // Eraser brush (vector edit mode).
    if let Some([x, y, r]) = ov.brush {
        let c = app.to_screen(x, y);
        let rs = (r * app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0)) as f32;
        painter.circle_stroke(c, rs, stroke(Color32::WHITE, 3.0));
        painter.circle_stroke(c, rs, stroke(accent(painter), 1.5));
    }
    // Lasso (vector edit mode).
    if ov.lasso.len() > 1 {
        let mut pts: Vec<Pos2> = ov.lasso.iter().map(|[x, y]| app.to_screen(*x, *y)).collect();
        if let Some(first) = pts.first().copied() {
            pts.push(first);
        }
        painter.add(Shape::dashed_line(&pts, stroke(accent(painter), 1.0), 4.0, 3.0));
    }
    // Guides (under the rulers).
    crate::guides::draw(app, painter);
    // Rulers.
    if view.rulers {
        rulers(app, painter, t, &doc, &sel);
    }
    labels
}

/// A transform box (screen corners tl, tr, br, bl): its outline when `outline`, the first `handles`
/// corner handles and, when dimensions are shown, the size `label` (if any) under it (its box returned).
fn transform_box(
    app: &FigApp,
    painter: &egui::Painter,
    q: [Pos2; 4],
    col: Color32,
    outline: bool,
    handles: usize,
    label: Option<String>,
) -> Option<Rect> {
    if outline {
        quad(painter, q, stroke(col, 1.0));
    }
    for c in q.iter().take(handles) {
        let r = Rect::from_center_size(*c, vec2(8.0, 8.0));
        painter.rect_filled(r, CornerRadius::same(1), Color32::WHITE);
        painter.rect_stroke(r, CornerRadius::same(1), stroke(col, 1.0), StrokeKind::Inside);
    }
    if let Some(label) = label.filter(|_| app.session.prefs.show_dimensions) {
        let bottom = q.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        let cx = q.iter().map(|p| p.x).sum::<f32>() / 4.0;
        return Some(pill(painter, pos2(cx, bottom + 9.0), Align2::CENTER_TOP, label, col));
    }
    None
}

/// A canvas readout: white `text` on a `col` pill placed at `at` by `align` (its box returned).
fn pill(painter: &egui::Painter, at: Pos2, align: Align2, text: String, col: Color32) -> Rect {
    let galley = painter.layout_no_wrap(text, semibold(11.0), Color32::WHITE);
    let r = align.anchor_size(at, galley.size() + vec2(8.0, 4.0));
    painter.rect_filled(r, CornerRadius::same(3), crate::theme::under_white(col));
    painter.galley(r.min + vec2(4.0, 2.0), galley, Color32::WHITE);
    r
}

/// The selected ellipse's arc handles (the hovered or dragged one larger, Start with a dot inside)
/// and, while one is dragged, its readout beside it.
fn arc_overlay(app: &FigApp, painter: &egui::Painter, col: Color32) {
    use figcraft_engine::arc_handles::ArcHandle;
    let Some(hs) = app.session.arc_handles() else { return };
    let ov = &app.session.tool.overlay;
    for h in [ArcHandle::Ratio, ArcHandle::Start, ArcHandle::Sweep] {
        let Some([x, y]) = hs.at(h) else { continue };
        let c = app.to_screen(x, y);
        let r = if ov.arc_hover == Some(h) { 5.0 } else { 4.0 };
        painter.circle_filled(c, r, Color32::WHITE);
        painter.circle_stroke(c, r, stroke(col, 1.0));
        if h == ArcHandle::Start {
            painter.circle_filled(c, 1.5, col);
        }
    }
    if let Some(hint) = &ov.arc {
        let c = app.to_screen(hint.at[0], hint.at[1]);
        pill(painter, c + vec2(12.0, 12.0), Align2::LEFT_TOP, hint.label.clone(), col);
    }
}

/// The selected shape's corner radius handles and a star's ratio handle (the hovered or dragged
/// one larger) and, while one is dragged, its value beside it.
fn shape_overlay(app: &FigApp, painter: &egui::Painter, col: Color32) {
    use figcraft_engine::shape_handles::ShapeHandle;
    let ov = &app.session.tool.overlay;
    for (h, [x, y]) in app.session.shape_handles() {
        let r = if ov.shape_hover == Some(h) { 4.5 } else { 3.5 };
        let c = app.to_screen(x, y);
        painter.circle_filled(c, r, Color32::WHITE);
        painter.circle_stroke(c, r, stroke(col, 1.0));
    }
    if let Some(hint) = &ov.shape {
        let c = app.to_screen(hint.at[0], hint.at[1]);
        let text = match hint.handle {
            ShapeHandle::Radius { .. } => format!("{} {}", tr("Radius"), hint.value),
            ShapeHandle::Ratio => format!("{} {}%", tr("Ratio"), hint.value),
        };
        pill(painter, c + vec2(12.0, 12.0), Align2::LEFT_TOP, text, col);
    }
}

/// Variable width tool: each width point on the path with a bar across the stroke to its two
/// width handles (the selected point filled), and a faint preview where a drag would add one.
fn width_overlay(app: &FigApp, painter: &egui::Painter, doc: &Document, ve: &figcraft_engine::vectoredit::VectorEdit, zoom: f64) {
    use figcraft_doc::WidthSide;
    use figcraft_engine::vectoredit::{WHit, width_handles, width_preview};
    let hover = app.session.tool.overlay.width_hover;
    let sp = |p: &[f64; 2]| app.to_screen(p[0], p[1]);
    let draw = |h: &figcraft_engine::vectoredit::WidthHandle, ghost: bool| {
        let (c, a, b) = (sp(&h.at), sp(&h.sides[0]), sp(&h.sides[1]));
        let col = if ghost { accent(painter).gamma_multiply(0.5) } else { accent(painter) };
        painter.line_segment([a, b], stroke(Color32::from_white_alpha(160), 3.0));
        painter.line_segment([a, b], stroke(col, 1.0));
        for (e, side) in [(a, WidthSide::Right), (b, WidthSide::Left)] {
            let grab = !ghost && hover == Some(WHit::Handle { index: h.index, side });
            let r = if grab { 4.5 } else { 3.5 };
            painter.circle_filled(e, r, Color32::WHITE);
            painter.circle_stroke(e, r, stroke(col, 1.0));
        }
        let on = !ghost && ve.width_point == Some(h.index);
        let hovered = !ghost && hover == Some(WHit::Point { index: h.index });
        let r = if hovered { 5.0 } else { 4.0 };
        painter.circle_filled(c, r, if on { accent(painter) } else { Color32::WHITE });
        painter.circle_stroke(c, r, stroke(col, 1.0));
    };
    for h in width_handles(doc, ve, zoom) {
        draw(&h, false);
    }
    if let Some(WHit::Path { t, .. }) = hover {
        for h in width_preview(doc, ve, t, zoom) {
            draw(&h, true);
        }
    }
}

/// A bezier handle from its anchor `v` to its end `e` (screen points): the line, and a dot of
/// radius `r` filled with `fill` (vector edit mode and the Pen draw them alike).
fn bezier_handle(painter: &egui::Painter, v: Pos2, e: Pos2, r: f32, fill: Color32) {
    painter.line_segment([v, e], stroke(accent(painter), 1.0));
    painter.circle_filled(e, r, fill);
    painter.circle_stroke(e, r, stroke(accent(painter), 1.0));
}

/// Vector edit mode: a layer's segments, handles and points (`points`) with its `hover`ed part, and
/// for the `active` layer the closed area under the paint bucket and the pen's rubber band.
fn vector_overlay(
    app: &FigApp,
    painter: &egui::Painter,
    o: &figcraft_engine::vectoredit::EditOverlay,
    points: bool,
    hover: Option<figcraft_engine::vectoredit::VHit>,
    active: bool,
) {
    use figcraft_engine::vectoredit::VHit;
    let ov = &app.session.tool.overlay;
    let sp = |p: &[f64; 2]| app.to_screen(p[0], p[1]);
    // Paint bucket target: hatched like a fill to come.
    if active && !ov.face.is_empty() {
        let polys: Vec<Vec<Pos2>> = ov.face.iter().map(|poly| poly.iter().map(sp).collect()).collect();
        hatched(painter, &polys, accent(painter));
    }
    if active {
        region_hatch(app, painter);
    }
    let hover_seg = match hover {
        Some(VHit::Segment { seg, .. }) => Some(seg),
        _ => None,
    };
    for (i, seg) in o.segments.iter().enumerate() {
        let [a, b, c, d] = [sp(&seg[0]), sp(&seg[1]), sp(&seg[2]), sp(&seg[3])];
        let w = if o.selected_segments.contains(&i) {
            3.0
        } else if hover_seg == Some(i) {
            2.5
        } else {
            1.5
        };
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke([a, b, c, d], false, Color32::TRANSPARENT, stroke(accent(painter), w)));
    }
    if !points {
        return;
    }
    for h in &o.handles {
        let hovered = hover == Some(VHit::Handle { seg: h.seg, start: h.start });
        let on = o.selected_handles.contains(&(h.seg, h.start));
        bezier_handle(painter, sp(&h.vertex), sp(&h.handle), if hovered { 4.5 } else { 3.5 }, if on { accent(painter) } else { Color32::WHITE });
    }
    for (i, v) in o.vertices.iter().enumerate() {
        let p = sp(v);
        let hovered = matches!(hover, Some(VHit::Vertex { index }) if index == i);
        let size = if hovered { 9.0 } else { 7.0 };
        let r = Rect::from_center_size(p, vec2(size, size));
        let on = o.selected_vertices.contains(&i);
        painter.rect_filled(r, CornerRadius::same(5), if on { accent(painter) } else { Color32::WHITE });
        painter.rect_stroke(r, CornerRadius::same(5), stroke(accent(painter), 1.0), StrokeKind::Inside);
        if o.anchor == Some(i) {
            painter.circle_stroke(p, 7.5, stroke(accent(painter), 1.0));
        }
    }
    if let Some([x0, y0, x1, y1]) = ov.rubber.filter(|_| active) {
        painter.add(Shape::dashed_line(&[app.to_screen(x0, y0), app.to_screen(x1, y1)], stroke(accent(painter), 1.0), 4.0, 3.0));
    }
}

/// Width of the lines hatching the hovered and selected regions in vector edit mode (screen px).
const REGION_HATCH_WIDTH: f32 = 1.4;

/// Vector edit mode: the selected regions and the hovered one hatched in the selection colour,
/// under the segments and points (the engine lays out and clips the lines).
fn region_hatch(app: &FigApp, painter: &egui::Painter) {
    let Ok(ds) = app.session.ds() else { return };
    let (ox, oy) = ds.view.page_to_screen(0.0, 0.0);
    let (z, min) = (ds.view.zoom, app.canvas.rect.min);
    let view = figcraft_geom::Affine::new([z, 0.0, 0.0, z, f64::from(min.x) + ox, f64::from(min.y) + oy]);
    let r = painter.clip_rect();
    let clip = figcraft_geom::Rect::new(f64::from(r.min.x), f64::from(r.min.y), f64::from(r.max.x), f64::from(r.max.y));
    for [a, b] in app.session.vector_region_hatch(view, Some(clip)) {
        painter.line_segment([pos2(a.x as f32, a.y as f32), pos2(b.x as f32, b.y as f32)], stroke(accent(painter), REGION_HATCH_WIDTH));
    }
}

/// Shape builder: every region's edges, and the hovered or collected regions hatched (in the
/// danger colour when releasing removes them).
fn builder_overlay(app: &FigApp, painter: &egui::Painter, t: &Tokens, arr: &figcraft_engine::shapebuilder::Arrangement) {
    let ov = &app.session.tool.overlay;
    let screen = |r: &figcraft_engine::shapebuilder::Region| -> Vec<Vec<Pos2>> {
        r.polys.iter().map(|poly| poly.iter().map(|p| app.to_screen(p.x, p.y)).collect()).collect()
    };
    // Edges read on any fill: a light halo under a thin accent line.
    for r in &arr.regions {
        for poly in screen(r) {
            painter.add(Shape::closed_line(poly.clone(), stroke(Color32::from_white_alpha(150), 2.5)));
            painter.add(Shape::closed_line(poly, stroke(accent(painter), 1.0)));
        }
    }
    let col = if ov.builder_remove { t.danger } else { accent(painter) };
    for r in ov.builder_regions.iter().filter_map(|i| arr.regions.get(*i)) {
        hatched(painter, &screen(r), col);
    }
}

/// Polygons hatched and outlined in `col` (an area about to be filled, merged or removed), each
/// line on a light halo so it reads on any fill, even one in `col`.
fn hatched(painter: &egui::Painter, polys: &[Vec<Pos2>], col: Color32) {
    let halo = Color32::from_white_alpha(200);
    for l in hatch(polys, 6.0) {
        painter.line_segment(l, stroke(halo, 3.0));
        painter.line_segment(l, stroke(col, 1.25));
    }
    for poly in polys {
        painter.add(Shape::closed_line(poly.clone(), stroke(halo, 4.0)));
        painter.add(Shape::closed_line(poly.clone(), stroke(col, 2.0)));
    }
}

/// 45° hatch lines `gap` apart inside polygons (even-odd), in screen space.
fn hatch(polys: &[Vec<Pos2>], gap: f32) -> Vec<[Pos2; 2]> {
    // Rotate so the hatch is horizontal, scan, rotate back.
    let rot = |p: Pos2| pos2((p.x + p.y) * std::f32::consts::FRAC_1_SQRT_2, (p.y - p.x) * std::f32::consts::FRAC_1_SQRT_2);
    let back = |x: f32, y: f32| pos2((x - y) * std::f32::consts::FRAC_1_SQRT_2, (x + y) * std::f32::consts::FRAC_1_SQRT_2);
    let edges: Vec<(Pos2, Pos2)> = polys
        .iter()
        .flat_map(|poly| {
            let pts: Vec<Pos2> = poly.iter().map(|p| rot(*p)).collect();
            let n = pts.len();
            (0..n).filter_map(move |k| Some((*pts.get(k)?, *pts.get((k + 1) % n)?)))
        })
        .collect();
    let (y0, y1) = edges.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), (a, b)| (lo.min(a.y).min(b.y), hi.max(a.y).max(b.y)));
    let mut out = Vec::new();
    if !(y0.is_finite() && y1.is_finite()) || (y1 - y0) / gap > 4000.0 {
        return out;
    }
    let mut y = (y0 / gap).ceil() * gap;
    while y < y1 {
        let mut xs: Vec<f32> = edges
            .iter()
            .filter(|(a, b)| (a.y <= y && y < b.y) || (b.y <= y && y < a.y))
            .map(|(a, b)| a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y))
            .collect();
        xs.sort_by(f32::total_cmp);
        for pair in xs.chunks_exact(2) {
            if let [a, b] = pair {
                out.push([back(*a, y), back(*b, y)]);
            }
        }
        y += gap;
    }
    out
}

/// The layers that get canvas labels and layout grids, in z-order: the page's visible top-level
/// layers and, inside sections whose contents show, their visible children (sections nest).
fn boards(doc: &Document, page: NodeId) -> Vec<NodeId> {
    fn walk(doc: &Document, id: NodeId, depth: usize, out: &mut Vec<NodeId>) {
        let Some(n) = doc.node(id) else { return };
        for c in &n.children {
            let Some(cn) = doc.node(*c) else { continue };
            if !cn.visible {
                continue;
            }
            out.push(*c);
            if matches!(cn.kind, NodeKind::Section { contents_hidden: false }) && depth < 32 {
                walk(doc, *c, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(doc, page, 0, &mut out);
    out
}

/// A frame's or section's handoff status as its badge says it, with the badge colour ("Changed"
/// once a ready-for-dev layer was edited; `None` without a status).
pub(crate) fn status_label(n: &figcraft_doc::Node) -> Option<(&'static str, Color32)> {
    match n.dev_status {
        figcraft_doc::DevStatus::None => None,
        figcraft_doc::DevStatus::ReadyForDev if n.dev_changed => Some((tr("Changed"), crate::theme::DEV_CHANGED)),
        figcraft_doc::DevStatus::ReadyForDev => Some((tr("Ready for dev"), crate::theme::DEV_READY)),
        figcraft_doc::DevStatus::Completed => Some((tr("Completed"), crate::theme::DEV_COMPLETED)),
    }
}

/// A frame's or section's handoff status as a small pill left-centred on `at`; the pill's area.
pub(crate) fn status_badge(painter: &egui::Painter, n: &figcraft_doc::Node, at: Pos2) -> Option<Rect> {
    let (label, fill) = status_label(n)?;
    let galley = painter.layout_no_wrap(label.to_string(), semibold(10.0), Color32::WHITE);
    let r = Rect::from_min_size(pos2(at.x, at.y - galley.size().y / 2.0 - 2.0), galley.size() + vec2(10.0, 4.0));
    painter.rect_filled(r, CornerRadius::same(4), fill);
    painter.galley(r.min + vec2(5.0, 2.0), galley, Color32::WHITE);
    Some(r)
}

fn rulers(app: &FigApp, painter: &egui::Painter, t: &Tokens, doc: &Document, sel: &[NodeId]) {
    let r = app.canvas.rect;
    let size = 20.0;
    painter.rect_filled(Rect::from_min_size(r.min, vec2(r.width(), size)), CornerRadius::ZERO, t.panel);
    painter.rect_filled(Rect::from_min_size(r.min, vec2(size, r.height())), CornerRadius::ZERO, t.panel);
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
    let step = figcraft_engine::guides::ruler_step(zoom);
    let p0 = app.to_page(r.min);
    let p1 = app.to_page(r.max);
    // Highlight the selection's span.
    if let Some(b) = doc.bounds_of(sel) {
        let (a, bb) = (app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1));
        painter.rect_filled(
            Rect::from_min_max(pos2(a.x, r.min.y), pos2(bb.x, r.min.y + size)),
            CornerRadius::ZERO,
            accent(painter).gamma_multiply(0.15),
        );
        painter.rect_filled(
            Rect::from_min_max(pos2(r.min.x, a.y), pos2(r.min.x + size, bb.y)),
            CornerRadius::ZERO,
            accent(painter).gamma_multiply(0.15),
        );
    }
    let mut x = (p0.x / step).floor() * step;
    while x <= p1.x {
        let s = app.to_screen(x, 0.0).x;
        painter.line_segment([pos2(s, r.min.y + size - 6.0), pos2(s, r.min.y + size)], stroke(t.text_tertiary, 1.0));
        painter.text(pos2(s + 2.0, r.min.y + 3.0), Align2::LEFT_TOP, fmt(x), regular(9.0), t.text_secondary);
        x += step;
    }
    let mut y = (p0.y / step).floor() * step;
    while y <= p1.y {
        let s = app.to_screen(0.0, y).y;
        painter.line_segment([pos2(r.min.x + size - 6.0, s), pos2(r.min.x + size, s)], stroke(t.text_tertiary, 1.0));
        painter.text(pos2(r.min.x + 3.0, s + 2.0), Align2::LEFT_TOP, fmt(y), regular(9.0), t.text_secondary);
        y += step;
    }
    painter.line_segment([pos2(r.min.x, r.min.y + size), pos2(r.max.x, r.min.y + size)], stroke(t.border, 1.0));
    painter.line_segment([pos2(r.min.x + size, r.min.y), pos2(r.min.x + size, r.max.y)], stroke(t.border, 1.0));
}

pub fn fmt(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-9 { format!("{}", r.round() as i64) } else { format!("{r}") }
}

#[cfg(test)]
#[path = "zoom_preview_tests.rs"]
mod zoom_preview_tests;

/// The overlay green for the page this painter draws on ([`crate::theme::canvas_accent`]).
fn accent(p: &egui::Painter) -> Color32 {
    crate::theme::canvas_accent(p.ctx())
}
