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

use std::sync::{Arc, Weak};

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
    /// The document the gesture started from (held, so no other document can take its address).
    base: Arc<Document>,
    /// The document and page the sprite belongs to: another one on screen ends it.
    of: (u64, NodeId),
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
    if moved.is_none() {
        app.canvas.sprite_declined = None;
    }
    if app.canvas.sprite.as_ref().is_some_and(|s| s.of != (ds.uid, ds.page)) {
        app.canvas.sprite = None;
    }
    let zoom = ds.view.zoom;
    let view_ok = view_allows(ds);
    if let Some(s) = app.canvas.sprite.as_mut().filter(|s| !s.ending) {
        if let Some((base, d)) = moved.as_ref().filter(|(b, _)| Arc::ptr_eq(b, &s.base))
            && zoom.to_bits() == s.zoom.to_bits()
            && ppp.to_bits() == s.ppp.to_bits()
            && view_ok
        {
            s.delta = *d;
            return Some((base.clone(), s.roots.clone()));
        }
        // The gesture ended or the plan stopped holding (then not again in this gesture): freeze
        // where the layers are now.
        if let Some((b, _)) = moved.as_ref().filter(|(b, _)| Arc::ptr_eq(b, &s.base)) {
            app.canvas.sprite_declined = Some(Arc::downgrade(b));
        }
        match s.roots.first().filter(|r| ds.doc.contains(**r)) {
            Some(r) => {
                let t = ds.doc.world_transform(*r).translation();
                s.delta = (t.x - s.root_at.0, t.y - s.root_at.1);
                s.ending = true;
                s.release = None;
            }
            None => app.canvas.sprite = None,
        }
    }
    let (base, _) = moved?;
    if app.canvas.sprite_declined.as_ref().is_some_and(|d| Weak::ptr_eq(d, &Arc::downgrade(&base))) || !view_ok {
        return None;
    }
    let canvas_px = f64::from(app.canvas.rect.width() * ppp) * f64::from(app.canvas.rect.height() * ppp);
    match plan(ctx, &base, &ds.selection, (ds.uid, ds.page), zoom, ppp, max_side, canvas_px) {
        Some(s) => {
            let out = (base, s.roots.clone());
            app.canvas.sprite = Some(s);
            Some(out)
        }
        None => {
            app.canvas.sprite_declined = Some(Arc::downgrade(&base));
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

/// The largest scale `figcraft_render::render_area` renders at (it clamps to this).
const RENDER_AREA_MAX_SCALE: f64 = 64.0;

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
    if n.parent.and_then(|p| doc.node(p)).is_none() {
        return false;
    }
    let mut at = n.parent;
    let mut child = id;
    while let Some(a) = at.and_then(|a| doc.node(a)) {
        // At every level, the page included: inside a boolean result, or above a mask (a mask
        // masks the layers above it in its container), the layer doesn't draw on its own.
        let above_a_mask = a.children.iter().take_while(|c| **c != child).any(|c| doc.node(*c).is_some_and(|c| c.is_mask));
        if matches!(a.kind, NodeKind::BooleanOp { .. }) || above_a_mask {
            return false;
        }
        if matches!(a.kind, NodeKind::Page { .. }) {
            break;
        }
        if !a.effects.is_empty() || !ordinary(a.blend) || a.opacity.is_nan() || a.opacity < 1.0 || a.is_mask {
            return false;
        }
        if a.clip && a.kind.is_frame_like() {
            // The sprite is clipped to a rectangle: a rounded frame clips to a shape it can't follow.
            if a.corners.radii.iter().any(|r| *r != 0.0) {
                return false;
            }
            let Some(b) = doc.world_bounds(a.id) else { return false };
            *clip = Some(clip.map_or(b, |c| c.intersect(b)));
        }
        child = a.id;
        at = a.parent;
    }
    true
}

/// Render the dragged layers of `base` into a sprite, if they qualify: the roots are plain (see
/// [`plain`]), none inside another, and the sprite fits a texture and a few canvases of pixels.
#[allow(clippy::too_many_arguments)]
fn plan(
    ctx: &egui::Context,
    base: &Arc<Document>,
    sel: &[NodeId],
    of: (u64, NodeId),
    zoom: f64,
    ppp: f32,
    max_side: u32,
    canvas_px: f64,
) -> Option<Sprite> {
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
    let roots: Vec<NodeId> = order.iter().copied().filter(|i| sel.contains(i)).collect();
    // Not all on the page of the first (or not in the document): the old way, rather than a guessed order.
    if roots.len() != sel.len() {
        return None;
    }
    let bounds = roots.iter().filter_map(|i| base.world_render_bounds(*i)).reduce(|a, b| a.union(b))?;
    let scale = zoom * f64::from(ppp);
    // `render_area` renders at most at this scale; past it the sprite would draw smaller than the layers.
    if scale > RENDER_AREA_MAX_SCALE {
        return None;
    }
    let (w, h) = ((bounds.width() * scale).ceil(), (bounds.height() * scale).ceil());
    let side = f64::from(max_side);
    if !(w >= 1.0 && h >= 1.0 && w <= side && h <= side && w * h <= canvas_px * MAX_AREA_FACTOR) {
        return None;
    }
    let pix = figcraft_render::render_area(base, &roots, bounds, scale)?;
    // What came back is what the texture is made of: the same limits again.
    let (pw, ph) = (f64::from(pix.width()), f64::from(pix.height()));
    if !(pw <= side && ph <= side && pw * ph <= canvas_px * MAX_AREA_FACTOR) {
        return None;
    }
    let view = figcraft_render::View { origin: (bounds.x0, bounds.y0), scale, width: pix.width(), height: pix.height() };
    let tex = ctx.load_texture("figcraft-drag-sprite", crate::render_worker::screen_image(&pix, base.color_profile), egui::TextureOptions::LINEAR);
    let t = base.world_transform(roots[0]).translation();
    Some(Sprite { tex, root_at: (t.x, t.y), roots, view, delta: (0.0, 0.0), zoom, ppp, base: base.clone(), of, clip, ending: false, release: None })
}

#[cfg(test)]
mod plain_tests {
    use super::plain;
    use figcraft_engine::Session;
    use serde_json::{Value, json};

    fn id(v: &Value) -> figcraft_doc::NodeId {
        figcraft_engine::params::id_of(&v["id"]).or_else(|| figcraft_engine::params::id_of(&v["ids"][0])).unwrap()
    }

    fn rect(s: &mut Session, x: f64) -> figcraft_doc::NodeId {
        id(&s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 40, "height": 40})).unwrap())
    }

    #[test]
    fn a_layer_in_a_group_inside_a_boolean_is_not_plain() {
        let mut s = Session::new();
        let (a, b, c) = (rect(&mut s, 0.0), rect(&mut s, 20.0), rect(&mut s, 60.0));
        let g = id(&s.execute("object.group", &json!({"ids": [a, b]})).unwrap());
        assert!(plain(s.doc().unwrap(), a, &mut None), "in a plain group it is plain");
        s.execute("bool.union", &json!({"ids": [g, c]})).unwrap();
        let doc = s.doc().unwrap();
        assert!(doc.ancestors(a).iter().any(|p| matches!(doc.node(*p).map(|n| &n.kind), Some(figcraft_doc::NodeKind::BooleanOp { .. }))));
        assert!(!plain(doc, a, &mut None), "its group sits in a boolean result");
    }

    #[test]
    fn a_layer_in_a_group_beside_a_mask_is_not_plain() {
        let mut s = Session::new();
        let m = rect(&mut s, 0.0);
        let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 30.0));
        let g = id(&s.execute("object.group", &json!({"ids": [a, b]})).unwrap());
        s.execute("object.mask", &json!({"ids": [m, g]})).unwrap();
        let doc = s.doc().unwrap();
        assert!(doc.node(m).is_some_and(|n| n.is_mask), "the bottom layer masks");
        assert!(!plain(doc, a, &mut None), "its group is masked");
    }

    fn frame_with_child(s: &mut Session, props: Value) -> (figcraft_doc::NodeId, figcraft_doc::NodeId) {
        let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap());
        s.execute("node.set", &json!({"id": f, "props": props})).unwrap();
        let c = id(&s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 40, "parent": f})).unwrap());
        (f, c)
    }

    #[test]
    fn what_the_sprite_cannot_stand_in_for_is_not_plain() {
        let mut s = Session::new();
        let (_, c) = frame_with_child(&mut s, json!({"clip": false}));
        assert!(plain(s.doc().unwrap(), c, &mut None), "a layer in a plain frame is plain");
        for props in [
            json!({"opacity": 0.5}),
            json!({"blend": "multiply"}),
            json!({"effects": [{"type": "dropShadow", "color": "#00000066", "offset": [0, 8], "radius": 24}]}),
        ] {
            let mut s = Session::new();
            let (f, c) = frame_with_child(&mut s, props.clone());
            let n = s.doc().unwrap().node(f).unwrap();
            assert!(n.opacity < 1.0 || !n.effects.is_empty() || n.blend != figcraft_color::BlendMode::PassThrough, "{props} took");
            assert!(!plain(s.doc().unwrap(), c, &mut None), "an ancestor with {props}");
        }
    }

    #[test]
    fn a_layers_own_opacity_is_drawn_by_the_sprite() {
        // `render_area` draws the layer as the page does, its opacity included.
        let mut s = Session::new();
        let r = rect(&mut s, 0.0);
        s.execute("node.set", &json!({"id": r, "props": {"opacity": 0.5}})).unwrap();
        assert!(s.doc().unwrap().node(r).unwrap().opacity < 1.0);
        assert!(plain(s.doc().unwrap(), r, &mut None));
    }

    #[test]
    fn only_the_layers_above_a_mask_are_masked() {
        // A mask set in place (one layer, no group made): it masks the layers above it in its
        // container, as `Document::paint_runs` draws it; the layers below it draw on their own.
        let mut s = Session::new();
        let (below, m, above) = (rect(&mut s, 0.0), rect(&mut s, 50.0), rect(&mut s, 100.0));
        s.execute("object.mask", &json!({"ids": [m]})).unwrap();
        let doc = s.doc().unwrap();
        assert!(doc.node(m).is_some_and(|n| n.is_mask), "the middle layer masks in place");
        let parent = |i| doc.node(i).and_then(|n| n.parent);
        assert!(parent(below) == parent(m) && parent(above) == parent(m), "one container");
        assert!(!plain(doc, above, &mut None), "above the mask: masked");
        assert!(plain(doc, below, &mut None), "below it nothing is masked");
    }

    #[test]
    fn a_rounded_clipping_ancestor_is_not_plain_a_square_one_is_clipped_to() {
        let mut s = Session::new();
        let (f, c) = frame_with_child(&mut s, json!({"clip": true}));
        let mut clip = None;
        assert!(plain(s.doc().unwrap(), c, &mut clip));
        assert_eq!(clip, s.doc().unwrap().world_bounds(f));
        let mut s = Session::new();
        let (_, c) = frame_with_child(&mut s, json!({"clip": true, "cornerRadius": 12}));
        assert!(!plain(s.doc().unwrap(), c, &mut None), "a rounded frame clips to a shape the sprite can't follow");
    }
}
