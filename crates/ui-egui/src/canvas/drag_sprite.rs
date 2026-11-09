//! The dragged-subtree sprite (#2033). The selection outline is drawn from the live document every
//! frame, but the canvas shows the last image the render worker finished, so while a layer is
//! dragged the content trails the outline by the render time of every pointer move. When a
//! gesture only translates plain layers ([`crate::selection_cache::moved`]), the layers are
//! rendered once into a texture of their own (the sprite) and drawn each frame at the same
//! shifted place as the outline, while the base render is made once without them
//! ([`figcraft_render::Options::skip`]) from the document the gesture started from, so no render
//! is requested again until the gesture ends.
//!
//! While it drags, the sprite draws above everything (the layers lose their z-order), blends and
//! backdrop effects against what lies under them are approximated, and a layer that doesn't
//! qualify (see [`plan`]) keeps the old way.

use std::sync::Arc;

use egui::{Color32, Rect, pos2};
use figcraft_color::BlendMode;
use figcraft_doc::{Document, NodeId, NodeKind};
use web_time::Instant;

use super::{CanvasState, FigApp};

/// The sprite is at most this many times the canvas in pixels.
const MAX_AREA_FACTOR: f64 = 4.0;

pub(super) struct Sprite {
    tex: egui::TextureHandle,
    /// The dragged layers.
    roots: Vec<NodeId>,
    /// Where the first of them was, in page units (the final shift is read off its move).
    root_at: (f64, f64),
    /// The page area the sprite shows when not shifted, and its pixels.
    view: figcraft_render::View,
    /// The pointer-driven shift (page units) while it drags, then the final one.
    delta: (f64, f64),
    zoom: f64,
    ppp: f32,
    /// The address of the document the gesture started from.
    base: usize,
    /// Page rectangle the layers are clipped to (the clipping frames above them).
    clip: Option<figcraft_geom::Rect>,
    /// The gesture is over (or the plan stopped holding): the sprite stays until the image of the
    /// document without a skip, asked for at or after `release`, is on screen.
    ending: bool,
    release: Option<Instant>,
}

impl Sprite {
    fn rect(&self, app: &FigApp, ppp: f32) -> Rect {
        let tl = app.to_screen(self.view.origin.0 + self.delta.0, self.view.origin.1 + self.delta.1);
        let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(self.zoom);
        super::presented_image_rect(tl, &self.view, zoom, ppp, false)
    }
}

/// Where the sprite shows on screen now, if there is one (tests compare it with the outline).
#[cfg(test)]
pub(super) fn screen_rect(app: &FigApp, ppp: f32) -> Option<Rect> {
    app.canvas.sprite.as_ref().map(|s| s.rect(app, ppp))
}

/// Draw the sprite, above the canvas image.
pub(super) fn draw(app: &FigApp, painter: &egui::Painter, ppp: f32) {
    let Some(s) = &app.canvas.sprite else { return };
    let r = s.rect(app, ppp);
    let painter = match s.clip {
        Some(c) => {
            let (a, b) = (app.to_screen(c.x0, c.y0), app.to_screen(c.x1, c.y1));
            painter.with_clip_rect(painter.clip_rect().intersect(Rect::from_two_pos(a, b)))
        }
        None => painter.clone(),
    };
    painter.image(s.tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}

/// A finished image of the canvas, asked for at `at`, is on screen: a sprite that is ending goes.
pub(super) fn arrived(c: &mut CanvasState, at: Instant, coarse: bool) {
    if !coarse && c.sprite.as_ref().is_some_and(|s| s.ending && s.release.is_some_and(|r| at >= r)) {
        c.sprite = None;
    }
}

/// A job without a skip was asked for at `at`: what an ending sprite waits for.
pub(super) fn released_at(c: &mut CanvasState, at: Instant) {
    if let Some(s) = c.sprite.as_mut().filter(|s| s.ending) {
        s.release = Some(at);
    }
}

/// An ending sprite no job waits on goes (nothing would ever replace it).
pub(super) fn drop_orphan(c: &mut CanvasState) {
    if c.sprite.as_ref().is_some_and(|s| s.ending && s.release.is_none()) {
        c.sprite = None;
    }
}

/// Bring the sprite in line with the gesture; the document and layers the canvas render is to be
/// made from, with the layers skipped, while it shows them.
pub(super) fn sync(app: &mut FigApp, ctx: &egui::Context, ppp: f32, max_side: u32) -> Option<(Arc<Document>, Vec<NodeId>)> {
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    let Ok(ds) = app.session.ds() else {
        app.canvas.sprite = None;
        return None;
    };
    let moved = crate::selection_cache::moved(ds);
    let zoom = ds.view.zoom;
    let view_ok = view_allows(ds);
    if let Some(s) = app.canvas.sprite.as_mut().filter(|s| !s.ending) {
        if let Some((base, d)) = moved.as_ref().filter(|(b, _)| Arc::as_ptr(b) as usize == s.base)
            && zoom.to_bits() == s.zoom.to_bits()
            && ppp.to_bits() == s.ppp.to_bits()
            && view_ok
        {
            s.delta = *d;
            return Some((base.clone(), s.roots.clone()));
        }
        // The gesture ended or the plan stopped holding (then not again in this gesture): freeze
        // where the layers are now.
        if let Some((b, _)) = moved.as_ref().filter(|(b, _)| Arc::as_ptr(b) as usize == s.base) {
            app.canvas.sprite_declined = Arc::as_ptr(b) as usize;
        }
        match ds.doc.world_transform(s.roots[0]).translation() {
            t if ds.doc.contains(s.roots[0]) => {
                s.delta = (t.x - s.root_at.0, t.y - s.root_at.1);
                s.ending = true;
                s.release = None;
            }
            _ => app.canvas.sprite = None,
        }
    }
    let (base, _) = moved?;
    let ptr = Arc::as_ptr(&base) as usize;
    if app.canvas.sprite_declined == ptr || !view_ok {
        return None;
    }
    let canvas_px = f64::from(app.canvas.rect.width() * ppp) * f64::from(app.canvas.rect.height() * ppp);
    match plan(ctx, &base, &ds.selection, zoom, ppp, max_side, canvas_px) {
        Some(s) => {
            let out = (base, s.roots.clone());
            app.canvas.sprite = Some(s);
            Some(out)
        }
        None => {
            app.canvas.sprite_declined = ptr;
            None
        }
    }
}

/// What the canvas shows that the sprite can't stand in for.
fn view_allows(ds: &figcraft_engine::DocState) -> bool {
    !ds.view.outlines
        && !ds.view.outlines_selection
        && !ds.view.outline_hidden
        && ds.focused().is_none()
        && ds.view.pixel_preview_scale().is_none()
        && !(ds.view.proof_srgb && !ds.doc.color_profile.is_srgb())
}

/// Normal, or the pass-through of groups and frames (which draws like it unless the layers in them blend).
fn ordinary(b: BlendMode) -> bool {
    matches!(b, BlendMode::Normal | BlendMode::PassThrough)
}

/// Whether layer `id` can be drawn apart from the rest: it and what it sits in are plain.
fn plain(doc: &Document, id: NodeId, clip: &mut Option<figcraft_geom::Rect>) -> bool {
    let Some(n) = doc.node(id) else { return false };
    if !n.effects.is_empty() || !ordinary(n.blend) || n.is_mask {
        return false;
    }
    let Some(parent) = n.parent.and_then(|p| doc.node(p)) else { return false };
    if matches!(parent.kind, NodeKind::BooleanOp { .. }) || parent.children.iter().any(|c| doc.node(*c).is_some_and(|c| c.is_mask)) {
        return false;
    }
    let mut at = Some(parent.id);
    while let Some(a) = at.and_then(|a| doc.node(a)) {
        if matches!(a.kind, NodeKind::Page { .. }) {
            break;
        }
        if !a.effects.is_empty() || !ordinary(a.blend) || a.opacity < 1.0 || a.is_mask {
            return false;
        }
        if a.clip && a.kind.is_frame_like() {
            let Some(b) = doc.world_bounds(a.id) else { return false };
            *clip = Some(clip.map_or(b, |c| c.intersect(b)));
        }
        at = a.parent;
    }
    true
}

/// Render the dragged layers of `base` into a sprite, if they qualify: the roots are plain (see
/// [`plain`]), none inside another, and the sprite fits a texture and a few canvases of pixels.
fn plan(ctx: &egui::Context, base: &Arc<Document>, sel: &[NodeId], zoom: f64, ppp: f32, max_side: u32, canvas_px: f64) -> Option<Sprite> {
    if sel.is_empty() || !zoom.is_finite() || zoom <= 0.0 {
        return None;
    }
    let mut clip = None;
    for id in sel {
        if !plain(base, *id, &mut clip) || sel.iter().any(|o| o != id && base.ancestors(*id).contains(o)) {
            return None;
        }
    }
    // Bottom to top, as the page draws them.
    let page = base.page_of(sel[0])?;
    let order = base.subtree(page);
    let mut roots: Vec<NodeId> = order.iter().copied().filter(|i| sel.contains(i)).collect();
    if roots.len() != sel.len() {
        roots = sel.to_vec();
    }
    let bounds = roots.iter().filter_map(|i| base.world_render_bounds(*i)).reduce(|a, b| a.union(b))?;
    let scale = zoom * f64::from(ppp);
    let (w, h) = ((bounds.width() * scale).ceil(), (bounds.height() * scale).ceil());
    let side = f64::from(max_side);
    if !(w >= 1.0 && h >= 1.0 && w <= side && h <= side && w * h <= canvas_px * MAX_AREA_FACTOR) {
        return None;
    }
    let pix = figcraft_render::render_area(base, &roots, bounds, scale)?;
    let view = figcraft_render::View { origin: (bounds.x0, bounds.y0), scale, width: pix.width(), height: pix.height() };
    let tex = ctx.load_texture("figcraft-drag-sprite", crate::render_worker::screen_image(&pix, base.color_profile), egui::TextureOptions::LINEAR);
    let t = base.world_transform(roots[0]).translation();
    Some(Sprite {
        tex,
        root_at: (t.x, t.y),
        roots,
        view,
        delta: (0.0, 0.0),
        zoom,
        ppp,
        base: Arc::as_ptr(base) as usize,
        clip,
        ending: false,
        release: None,
    })
}
