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

/// A canvas texture registered with egui: its generation, egui's id, its size and the filter it
/// was registered with.
#[cfg(not(target_arch = "wasm32"))]
type GpuTexture = (u64, egui::TextureId, (u32, u32), wgpu::FilterMode);

mod builder;
mod input;
mod outline;
mod overlays;
mod render;
mod rulers;

#[cfg(test)]
use builder::clean_poly;
use input::input;
pub use input::open_context_menu;
pub(crate) use outline::{TextOnScreen, node_quad};
use overlays::{accent, overlays};
use render::{JobKey, request_render, shows};
pub use rulers::fmt;
pub(crate) use rulers::{status_badge, status_label};
pub mod board_bar;
pub mod board_connect;
pub mod board_facilitate;
pub mod comments;
pub mod cursors;
pub mod frame_labels;
pub mod frame_stats;
pub mod guides;
pub mod handles;
pub mod live_text;
pub mod measure_overlay;
pub mod proto_canvas;
pub mod render_worker;
pub(crate) mod selection_spacing;
pub mod widget_bar;

pub struct CanvasState {
    pub rect: Rect,
    texture: Option<egui::TextureHandle>,
    /// The window's wgpu state, for showing the GPU canvas's texture (desktop).
    #[cfg(not(target_arch = "wasm32"))]
    pub render_state: Option<egui_wgpu::RenderState>,
    /// The texture the GPU canvas drew the last frame into; shown instead of `texture` when set.
    #[cfg(not(target_arch = "wasm32"))]
    gpu_tex: Option<egui::TextureId>,
    /// The part of that texture the frame fills (`[0, 0, 1, 1]` for a full frame; an interaction
    /// preview fills the canvas's top-left part).
    #[cfg(not(target_arch = "wasm32"))]
    gpu_uv: [f32; 4],
    /// The textures registered with egui, by canvas buffer.
    #[cfg(not(target_arch = "wasm32"))]
    gpu_ids: [Option<GpuTexture>; 2],
    tex_view: Option<figcraft_render::View>,
    /// The file ([`figcraft_engine::DocState::uid`]) and page the texture shows: it is drawn only
    /// on them.
    tex_of: Option<(u64, NodeId)>,
    /// The texture is a coarse preview (see [`crate::render_worker`]).
    pub tex_coarse: bool,
    /// The document revision the texture shows (a preview of newer content goes on over it).
    tex_revision: Option<u64>,
    /// Which rasterizer drew the shown frame: `"gpu"`, `"cpu"` (or `"none"` before the first one).
    pub rasterizer: &'static str,
    /// The request the texture answers ([`crate::render_worker::Done::seq`]).
    tex_seq: u64,
    submitted: Option<JobKey>,
    /// When the last render was asked for (a preview is sharpened only once nothing has been
    /// asked for for [`crate::render_worker::GPU_REFINE_AFTER_MS`]).
    last_submitted: Option<web_time::Instant>,
    /// The key a sharp frame was asked for over a preview (so it is asked for once).
    refined: Option<JobKey>,
    /// The view of the last render asked for.
    want_view: Option<figcraft_render::View>,
    /// What the last canvas render had to do (shown by `ui.inspect`).
    pub render_work: Option<figcraft_render::Work>,
    /// From asking for a render to its preview, and its refined image, handed to egui as the
    /// canvas texture (ms; the texture is uploaded when that frame is painted).
    pub preview_latency_ms: Option<f64>,
    pub refine_latency_ms: Option<f64>,
    /// Frames shown so far, previews and refined ones apart (`ui.inspect`: how an interaction
    /// keeps up with the pointer).
    pub preview_frames: u64,
    pub refined_frames: u64,
    dragging: bool,
    space_pan: bool,
    middle_pan: Option<(Pos2, f64, f64)>,
    /// The zoom tool (Z held): zooming out (Alt) or in, and where its drag started.
    zoom_tool: Option<bool>,
    zoom_drag: Option<Pos2>,
    pub last_pointer: Option<Point>,
    /// The real pointer was over the canvas last frame (leaving it clears what it hovered).
    hovering: bool,
    /// The hover last sent to the tools and the state it was found in ([`input::HoverKey`]): the
    /// frames egui draws after a pointer move carry no new input, and would find the same again.
    last_hover: Option<input::HoverKey>,
    /// What the pointer shows over the canvas (`ui.inspect`); none while it is elsewhere or
    /// something on the canvas with a cursor of its own has it (a ruler guide, a gradient handle).
    pub cursor: Option<crate::cursors::Shown>,
    preedit: Option<PreeditView>,
    /// The pen pressure last reported during the current press (pen and stylus touches; the
    /// winit backends and browsers that report it).
    pressure: Option<f32>,
    /// The latest scroll came from a mouse wheel rather than a trackpad (Scroll wheel zoom).
    wheel_is_mouse: bool,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            rect: Rect::NOTHING,
            texture: None,
            #[cfg(not(target_arch = "wasm32"))]
            render_state: None,
            #[cfg(not(target_arch = "wasm32"))]
            gpu_tex: None,
            #[cfg(not(target_arch = "wasm32"))]
            gpu_uv: [0.0, 0.0, 1.0, 1.0],
            #[cfg(not(target_arch = "wasm32"))]
            gpu_ids: [None, None],
            tex_view: None,
            tex_of: None,
            tex_coarse: false,
            tex_revision: None,
            rasterizer: "none",
            tex_seq: 0,
            submitted: None,
            last_submitted: None,
            refined: None,
            want_view: None,
            render_work: None,
            preview_latency_ms: None,
            refine_latency_ms: None,
            preview_frames: 0,
            refined_frames: 0,
            dragging: false,
            space_pan: false,
            middle_pan: None,
            cursor: None,
            zoom_tool: None,
            zoom_drag: None,
            last_pointer: None,
            hovering: false,
            last_hover: None,
            preedit: None,
            pressure: None,
            wheel_is_mouse: false,
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
        // Crop mode: resizing the layer crops it, as Cmd/Ctrl-resizing does.
        cmd: m.command || app.ui.crop.is_some(),
        ctrl: m.ctrl,
        zoom,
        pressure: app.canvas.pressure,
    }
}

/// Run one canvas pass: update the viewport size and first-fit zoom; handle pointer/key input
/// and accessibility; draw the drop overlay, guides, live pointer and preedit; request/poll
/// renders and upload the texture; draw the page background and last texture; then draw overlays,
/// labels, handles, comments, versions/branch/dev/live UI and the cursor.
pub fn show(app: &mut FigApp, ui: &mut Ui, worker: &mut Worker) {
    let rect = ui.max_rect();
    app.canvas.rect = rect;
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    let resp = ui.interact(rect, ui.scope_id().with("canvas"), Sense::click_and_drag());
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
    // A browser names no dragged file until the drop, so documents there still say "import", or
    // in Dev Mode show the Dev Mode note, though they open when dropped.
    let documents = ctx.input(|i| i.raw.hovered_files.iter().all(|f| f.path.as_deref().is_some_and(crate::is_document)));
    let label = if documents {
        tr("Drop to open")
    } else if app.ui.right_tab == crate::RightTab::Inspect {
        tr("Dev Mode only inspects: switch to Design (Shift+D) to edit")
    } else {
        tr("Drop to import")
    };
    crate::drop_overlay(&ctx, ctx.content_rect(), label);
    crate::guides::end_released(&ctx);
    crate::live::track_pointer(app, &ctx);
    app.sync_preedit();
    // Retained rendering snaps the view outward and adds one pixel on each axis.
    let max_texture_side = (ctx.input(|i| i.max_texture_side).min(16_384) as u32).saturating_sub(1).max(1);
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
    let dots = crate::board_bar::dots(app, bg, &t);
    // A pointer held down is an interaction in progress: its previews are not sharpened between
    // its steps (a full frame would cost the drag its next preview).
    let holding = ctx.input(|i| i.pointer.any_down());
    if let Some(wait) = request_render(app, worker, ppp, max_texture_side, dots, holding) {
        ctx.request_repaint_after(wait);
    }
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
        let pixel_preview = app.session.ds().ok().and_then(|d| d.view.pixel_preview_scale()) == Some(done.view.scale);
        // The GPU canvas hands over its texture instead of an image: register it (once per size)
        // and show it; otherwise the image goes up as egui's own texture.
        #[cfg(target_arch = "wasm32")]
        let drawn_on_gpu = false;
        #[cfg(not(target_arch = "wasm32"))]
        let drawn_on_gpu = match done.gpu.as_ref() {
            Some(frame) => {
                let filter = if pixel_preview { wgpu::FilterMode::Nearest } else { wgpu::FilterMode::Linear };
                app.canvas.gpu_tex = register_gpu_texture(app, frame, filter);
                app.canvas.gpu_uv = frame.uv;
                app.canvas.gpu_tex.is_some()
            }
            None => {
                app.canvas.gpu_tex = None;
                app.canvas.gpu_uv = [0.0, 0.0, 1.0, 1.0];
                false
            }
        };
        if !drawn_on_gpu {
            let filter = if pixel_preview { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR };
            match &mut app.canvas.texture {
                Some(t) => t.set(done.image, filter),
                None => app.canvas.texture = Some(ctx.load_texture("figcraft-canvas", done.image, filter)),
            }
        }
        let latency = Some(done.at.elapsed().as_secs_f64() * 1000.0);
        if done.coarse {
            app.canvas.preview_latency_ms = latency;
            app.canvas.preview_frames = app.canvas.preview_frames.saturating_add(1);
        } else {
            app.canvas.refine_latency_ms = latency;
            app.canvas.refined_frames = app.canvas.refined_frames.saturating_add(1);
        }
        app.canvas.tex_view = Some(done.view);
        app.canvas.tex_of = Some((done.uid, done.page));
        app.canvas.tex_coarse = done.coarse;
        app.canvas.tex_revision = Some(done.revision);
        app.canvas.rasterizer = if drawn_on_gpu { "gpu" } else { "cpu" };
        app.canvas.tex_seq = done.seq;
        if done.coarse {
            // One more frame: this one asked for a render before the preview was on screen, and
            // that next frame asks (with nothing new to draw) for the sharp one.
            ctx.request_repaint();
        } else {
            app.canvas.refined = None;
        }
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, bg);
    crate::theme::set_canvas_page(&ctx, bg);
    if let (Some(v), true) = (&app.canvas.tex_view, shown.is_some() && app.canvas.tex_of == shown) {
        // Re-project the last render onto the current view.
        let tl = app.to_screen(v.origin.0, v.origin.1);
        let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
        let pixel_preview = app.session.ds().ok().and_then(|ds| ds.view.pixel_preview_scale()) == Some(v.scale);
        let image_rect = presented_image_rect(tl, v, zoom, ppp, pixel_preview);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        #[cfg(not(target_arch = "wasm32"))]
        let gpu = app.canvas.gpu_tex;
        #[cfg(not(target_arch = "wasm32"))]
        let gpu_uv = app.canvas.gpu_uv;
        #[cfg(target_arch = "wasm32")]
        let gpu: Option<egui::TextureId> = None;
        if let Some(id) = gpu {
            // A preview fills the canvas texture's top-left part: sample just that.
            #[cfg(not(target_arch = "wasm32"))]
            let uv = Rect::from_min_max(pos2(gpu_uv[0], gpu_uv[1]), pos2(gpu_uv[2], gpu_uv[3]));
            painter.image(id, image_rect, uv, Color32::WHITE);
        } else if let Some(tex) = &app.canvas.texture {
            painter.image(tex.id(), image_rect, uv, Color32::WHITE);
        }
    }
    crate::dev_mode::refresh_measurements(app);
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
    crate::board_facilitate::badges(app, &painter);
    crate::board_bar::stamp_author(app, &painter, ctx.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p)));
    crate::board_bar::show(app, &ctx);
    crate::board_connect::show(app, &ctx);
    crate::board_facilitate::show(app, &ctx, rect);
    crate::widget_bar::show(app, &ctx);
    crate::selection_spacing::draw(app, &painter);
    crate::comments::draw(app, &painter);
    crate::live_text::draw(app, &painter);
    crate::live::draw(app, &painter, &ctx);
    crate::live::agent::draw(app, &painter, &ctx);
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

/// Register the GPU canvas's buffer with egui (once per buffer and generation), freeing the
/// texture it replaces (which the canvas has already reallocated).
#[cfg(not(target_arch = "wasm32"))]
fn register_gpu_texture(app: &mut FigApp, frame: &figcraft_gpu::canvas::Frame, filter: wgpu::FilterMode) -> Option<egui::TextureId> {
    let state = app.canvas.render_state.clone()?;
    let slot = usize::from(frame.slot).min(app.canvas.gpu_ids.len().saturating_sub(1));
    // The texture's own size, not the frame's: a preview fills part of the same canvas texture
    // (and says which part in `frame.uv`), so registering it once per generation is enough.
    let size = frame.texture_size;
    if let Some((generation, id, cached, cached_filter)) = app.canvas.gpu_ids.get(slot).copied().flatten()
        && generation == frame.generation
        && cached == size
        && cached_filter == filter
    {
        return Some(id);
    }
    let mut renderer = state.renderer.write();
    if let Some((_, old, _, _)) = app.canvas.gpu_ids.get(slot).copied().flatten() {
        renderer.free_texture(&old);
    }
    let id = renderer.register_native_texture(&state.device, &frame.texture, filter);
    if let Some(slot) = app.canvas.gpu_ids.get_mut(slot) {
        *slot = Some((frame.generation, id, size, filter));
    }
    Some(id)
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

#[cfg(test)]
mod builder_poly_tests;

#[cfg(test)]
mod outline_tests;

#[cfg(test)]
mod selection_cache_tests;

#[cfg(test)]
mod wheel_zoom_tests;

#[cfg(test)]
mod zoom_preview_tests;
