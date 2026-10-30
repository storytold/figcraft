//! Document → vello scene: what the GPU canvas draws (see `docs/gpu-rasterizer.md`).
//!
//! Geometry is the renderer's: [`figcraft_render::node_outline`] builds a node's outline (shapes,
//! vectors, booleans and text as glyph outlines), so both rasterizers draw the same paths. This
//! module only turns outlines and paints into vello commands, and refuses — with a reason — what
//! it cannot draw exactly yet, which makes the canvas fall back to the CPU for that frame.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use figcraft_color::{BlendMode, Color, Gradient, GradientKind, Paint, PaintKind};
use figcraft_doc::{Document, Node, NodeId, NodeKind};
use figcraft_geom::{Affine, Shape as _};
use figcraft_render::paint::{Prepared, prepare_at};
use figcraft_render::{FillRule, Pixmap, View, node_outline};
use tiny_skia::{FilterQuality, SpreadMode, Transform};
use vello::Scene;
use vello::kurbo::{Affine as KAffine, Point, Rect as KRect, Stroke};
use vello::peniko::color::DynamicColor;
use vello::peniko::{
    BlendMode as PBlend, Blob, Brush, Color as PColor, ColorStop, ColorStops, Compose, Extend, Fill, Gradient as PGradient, ImageAlphaType,
    ImageBrush, ImageData, ImageFormat, ImageQuality, ImageSampler, Mix,
};

/// Why a page can't be drawn on the GPU yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// A visible effect (shadow, blur, glass, grain): drawn by GPU passes in a later phase.
    Effect,
    /// A paint the scene builder doesn't draw exactly (a skewed linear gradient, plus-darker).
    Paint,
    /// A stroke shape it doesn't draw yet (inside/outside, dashes, variable width, brushes, arrows).
    Stroke,
    /// A mask (its pixels, or how it covers its siblings).
    Mask,
    /// Clipping, opacity or blend in a place the builder doesn't handle.
    Structure,
}

impl Unsupported {
    /// A short reason for logs.
    pub fn reason(self) -> &'static str {
        match self {
            Unsupported::Effect => "an effect",
            Unsupported::Paint => "a paint",
            Unsupported::Stroke => "a stroke",
            Unsupported::Mask => "a mask",
            Unsupported::Structure => "a structure",
        }
    }
}

/// Deepest nesting drawn (the renderer's export cap).
const MAX_DEPTH: usize = 64;

/// What a node's backdrop effect is, by its id (the canvas makes its pixels from the accumulated
/// page).
pub fn backdrop_of_node(doc: &Document, id: NodeId) -> Option<Backdrop> {
    backdrop_of(doc.node(id)?)
}

/// A background-blurred node's layer: the device rectangle of its backdrop (the layer grown by
/// `pad` each way) — what the CPU blurs, and therefore the region the canvas crops and blurs.
/// Clipped to the view: the crop reads the accumulated page, which is the view.
pub fn blur_rect(doc: &Document, id: NodeId, view: &View) -> Option<((f64, f64), (u32, u32))> {
    let n = doc.node(id)?;
    let (x0, y0, w, h) = layer_rect_impl(doc, n, view).ok()??;
    let pad = f64::from(figcraft_render::backdrop::pad(n, view.scale));
    let x = (x0 - pad).max(0.0);
    let y = (y0 - pad).max(0.0);
    let w = ((x0 + f64::from(w) + pad).min(f64::from(view.width)) - x).clamp(1.0, 16_384.0) as u32;
    let h = ((y0 + f64::from(h) + pad).min(f64::from(view.height)) - y).clamp(1.0, 16_384.0) as u32;
    Some(((x, y), (w, h)))
}

/// A glass node's pane geometry and its layer's device position, for the canvas's crop.
pub fn glass_setup(doc: &Document, id: NodeId, view: &View) -> Option<(figcraft_render::glass::GlassPane, (i32, i32))> {
    let n = doc.node(id)?;
    let Backdrop::Glass(g) = backdrop_of(n)? else { return None };
    let (x0, y0, w, h) = layer_rect_impl(doc, n, view).ok()??;
    let parent = n.parent.map(|p| doc.world_transform(p)).unwrap_or(Affine::IDENTITY);
    let xf = view.transform() * parent * n.transform;
    let shape = figcraft_render::backdrop::outline(doc, n)?;
    let layer_xf = Affine::translate((-x0, -y0)) * xf;
    let mut pane = figcraft_render::glass::pane(&g, &shape, w, h, layer_xf, view.scale)?;
    pane.pad = figcraft_render::backdrop::pad(n, view.scale);
    Some((pane, (x0 as i32, y0 as i32)))
}

/// The device rectangle of `n`'s effect layer, in whole pixels (`None` past the layer cap). The
/// renderer's own layer rectangle: its paint bounds floored to a pixel, without its effects'
/// margin — the crop a backdrop effect reads, and the grid the glass geometry is built on. Clipped
/// to the view, as the renderer clips its layers: what is off the window is not drawn.
fn layer_rect_impl(doc: &Document, n: &Node, view: &View) -> Result<Option<(f64, f64, u32, u32)>, Unsupported> {
    let Some(bounds) = figcraft_render::layer_bounds(doc, n.id) else { return Ok(None) };
    let (x0f, y0f) = ((bounds.x0 - view.origin.0) * view.scale, (bounds.y0 - view.origin.1) * view.scale);
    let (x1f, y1f) = ((bounds.x1 - view.origin.0) * view.scale, (bounds.y1 - view.origin.1) * view.scale);
    if !(x0f.is_finite() && y0f.is_finite() && x1f.is_finite() && y1f.is_finite()) {
        return Err(Unsupported::Structure);
    }
    let (x0, y0) = (x0f.floor().max(0.0), y0f.floor().max(0.0));
    // A backdrop node off the view still crops a pixel of it: its effect draws where the node is
    // (off the canvas), and refusing here would leave the whole frame on the CPU.
    let (x1, y1) = (x1f.ceil().min(f64::from(view.width)).max(x0 + 1.0), y1f.ceil().min(f64::from(view.height)).max(y0 + 1.0));
    let (w, h) = (x1 - x0, y1 - y0);
    const MAX_LAYER: f64 = 8_192.0;
    if w > MAX_LAYER || h > MAX_LAYER {
        return Err(Unsupported::Effect);
    }
    Ok(Some((x0, y0, w as u32, h as u32)))
}

/// The device rectangle of `n`'s layer at its **exact** device corner, with a size that does not
/// depend on where the node sits: its paint bounds' device extent rounded up, plus a pixel of
/// slack for the corner's fraction. A layer the frame makes (a blur, noise, texture) draws its
/// content in the layer's own space from this corner, so a drag by part of a device pixel leaves
/// the content where it is and moves the layer — where the floored corner put that fraction into
/// the content, and every subpixel move re-blurred it (see `layer_pipeline`).
///
/// Clipped to the view, as the renderer clips its layers: a layer bigger than the window is mostly
/// offscreen, and blurring it would cost the device pixels that never show. This is what keeps any
/// zoom within budget — a 200 × 300 card's layer is 1400 × 1466 at 100%, 8400 × 9000 at 600%, and
/// 2280 × 1624 (the view) or less at 600% now.
fn layer_rect_exact_impl(doc: &Document, n: &Node, view: &View) -> Result<Option<(f64, f64, u32, u32)>, Unsupported> {
    let Some(bounds) = figcraft_render::layer_bounds(doc, n.id) else { return Ok(None) };
    let (x0f, y0f) = ((bounds.x0 - view.origin.0) * view.scale, (bounds.y0 - view.origin.1) * view.scale);
    let (x1f, y1f) = ((bounds.x1 - view.origin.0) * view.scale, (bounds.y1 - view.origin.1) * view.scale);
    if !(x0f.is_finite() && y0f.is_finite() && x1f.is_finite() && y1f.is_finite()) {
        return Err(Unsupported::Structure);
    }
    let (x0, y0) = (x0f.max(0.0), y0f.max(0.0));
    let (x1, y1) = (x1f.min(f64::from(view.width)), y1f.min(f64::from(view.height)));
    if !(x1 > x0 && y1 > y0) {
        return Ok(None);
    }
    // The corner stays exact (a clipped corner is the view's edge); the slack pixel may stick out.
    let (w, h) = ((x1 - x0).ceil() + 1.0, (y1 - y0).ceil() + 1.0);
    const MAX_LAYER: f64 = 8_192.0;
    if w > MAX_LAYER || h > MAX_LAYER {
        return Err(Unsupported::Effect);
    }
    Ok(Some((x0, y0, w as u32, h as u32)))
}

/// The page's background colour, drawn under the scene.
pub fn background(doc: &Document, page: NodeId) -> Color {
    match doc.node(page).map(|p| &p.kind) {
        Some(NodeKind::Page { background, .. }) => *background,
        _ => Color::DEFAULT_PAGE,
    }
}

/// What a node's backdrop effect is (one glass pane, or one uniform background blur).
pub enum Backdrop {
    Blur(f64),
    Glass(figcraft_doc::Glass),
}

fn backdrop_of(n: &Node) -> Option<Backdrop> {
    let mut visible = n.effects.iter().filter(|e| e.visible);
    let e = visible.next()?;
    if visible.next().is_some() {
        return None;
    }
    match &e.kind {
        figcraft_doc::EffectKind::BackgroundBlur(b) if !b.is_progressive() && b.max_radius() > 0.0 => Some(Backdrop::Blur(b.sanitized().radius)),
        figcraft_doc::EffectKind::Glass(g) => Some(Backdrop::Glass(g.clone())),
        _ => None,
    }
}

/// One step of a page drawn in segments: ordinary layers, or a node that reads the page under it.
pub enum Step {
    Nodes(Vec<NodeId>),
    /// A glass pane or a background-blurred layer.
    Backdrop(NodeId),
    /// A node whose blend is plus-darker: its own pixels, composited by a pass of their own (vello
    /// has no such blend mode).
    PlusDarker(NodeId),
}

/// How to draw `page` in segments: its top-level children in order, with backdrop nodes as steps
/// of their own (the page is accumulated up to each one). A container *with* one nested inside it
/// gets a step of its own too (the builder re-renders what lies under it, which is work of its
/// own, and the layers before it can then be kept between frames); a backdrop node *inside* a
/// layer would need a boundary there, which would be wrong — the builder draws it whole instead.
pub fn plan(doc: &Document, page: NodeId) -> Result<Vec<Step>, Unsupported> {
    let Some(p) = doc.node(page) else { return Ok(Vec::new()) };
    let mut steps = Vec::new();
    let mut run: Vec<NodeId> = Vec::new();
    for c in &p.children {
        let Some(n) = doc.node(*c) else { continue };
        if !n.visible || n.opacity <= 0.0 {
            continue;
        }
        if backdrop_of(n).is_some() {
            if !run.is_empty() {
                steps.push(Step::Nodes(std::mem::take(&mut run)));
            }
            steps.push(Step::Backdrop(*c));
        } else if n.blend == BlendMode::PlusDarker {
            // Its pixels are composited by a pass of their own (vello has no plus-darker).
            if !run.is_empty() {
                steps.push(Step::Nodes(std::mem::take(&mut run)));
            }
            steps.push(Step::PlusDarker(*c));
        } else if nests_backdrop(doc, n, 0) {
            if !run.is_empty() {
                steps.push(Step::Nodes(std::mem::take(&mut run)));
            }
            steps.push(Step::Nodes(vec![*c]));
        } else {
            run.push(*c);
        }
    }
    if !run.is_empty() {
        steps.push(Step::Nodes(run));
    }
    Ok(steps)
}

/// The key of a step's own pixels (`Step::Nodes`): its nodes and everything under them, the view
/// and the zoom — plus the document's shape and text layout, as a shadow's key — so a drag re-draws
/// only the steps it changed. The nodes' own placements are part of it: a step's pixels move with
/// them (a layer's *texture* may not, see [`content_hash`]).
pub fn step_key(doc: &Document, view: &View, ids: &[NodeId]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for id in ids {
        if let Some(n) = doc.node(*id) {
            node_hash(doc, n, 0, true).hash(&mut h);
        }
    }
    (doc.next_id(), doc.node_count()).hash(&mut h);
    figcraft_text::layout_generation().hash(&mut h);
    doc.color_profile.hash(&mut h);
    for v in [view.origin.0, view.origin.1, view.scale].into_iter().map(f64::to_bits) {
        v.hash(&mut h);
    }
    (view.width, view.height).hash(&mut h);
    h.finish()
}

/// Whether a visible backdrop effect (glass, background blur) is nested somewhere under `n`.
fn nests_backdrop(doc: &Document, n: &Node, depth: usize) -> bool {
    if depth > MAX_DEPTH || !n.visible {
        return false;
    }
    n.children.iter().any(|c| doc.node(*c).is_some_and(|cn| (cn.visible && backdrop_of(cn).is_some()) || nests_backdrop(doc, cn, depth + 1)))
}

/// Whether a step's pixels depend on its own nodes alone: a backdrop effect nested inside the
/// step reads what lies under it, which may be another step's layers, so such a step is drawn
/// every frame instead of being kept.
pub fn step_is_self_contained(doc: &Document, ids: &[NodeId]) -> bool {
    !ids.iter().filter_map(|id| doc.node(*id)).any(|n| n.visible && nests_backdrop(doc, n, 0))
}

/// Draw `ids` (children of `page`, in order) into `out`, a page-level node at a time: each node's
/// encoded scene is kept between frames under its key, so a drag re-encodes only what it changed.
pub fn draw_into(doc: &Document, page: NodeId, view: &View, layers: &mut dyn Layers, ids: &[NodeId], out: &mut Scene) -> Result<(), Unsupported> {
    for id in ids {
        draw_top(doc, page, view, layers, *id, out, false)?;
    }
    Ok(())
}

/// Draw a plus-darker node's own pixels into `out`, its blend left for the canvas to apply (vello
/// has no plus-darker): the step is composited with a pass of its own.
pub fn draw_plus_into(doc: &Document, page: NodeId, view: &View, layers: &mut dyn Layers, id: NodeId, out: &mut Scene) -> Result<(), Unsupported> {
    draw_top(doc, page, view, layers, id, out, true)
}

/// Draw one child of `page` into `out` — its kept encoded scene when `layers` has it under the
/// node's key, else encoded now and kept. `as_normal` draws a plus-darker node's pixels without
/// its blend: the canvas composites them in a pass of its own ([`draw_plus_into`]).
fn draw_top(
    doc: &Document,
    page: NodeId,
    view: &View,
    layers: &mut dyn Layers,
    id: NodeId,
    out: &mut Scene,
    as_normal: bool,
) -> Result<(), Unsupported> {
    let key = node_scene_key(doc, view, id);
    if let Some(key) = key
        && let Some(kept) = layers.cached_node(key)
    {
        out.append(&kept, None);
        return Ok(());
    }
    let mut b = Builder {
        doc,
        scene: Some(Scene::new()),
        layers,
        view: view.clone(),
        page,
        hidden: Vec::new(),
        no_strokes: Vec::new(),
        as_content: Vec::new(),
        blend_as_normal: if as_normal { vec![id] } else { Vec::new() },
    };
    b.node(id, view.transform(), 0)?;
    let Some(scene) = b.scene.take() else { return Ok(()) };
    if let Some(key) = key {
        let scene = Arc::new(scene);
        layers.keep_node(key, Arc::clone(&scene));
        out.append(&scene, None);
    } else {
        out.append(&scene, None);
    }
    Ok(())
}

/// The key of a page-level node's encoded scene: its subtree, the view and the zoom — or `None`
/// when its pixels are not its own (a backdrop effect reads the page under it, which may change
/// without the node changing).
pub(crate) fn node_scene_key(doc: &Document, view: &View, id: NodeId) -> Option<u64> {
    let n = doc.node(id)?;
    if !n.visible || n.opacity <= 0.0 || n.is_mask || reads_backdrop(doc, n) || has_effects(doc, n) {
        return None;
    }
    Some(step_key(doc, view, &[id]))
}

/// Whether anything under `n` draws a layer image (a blur, grain, texture, inner shadow, backdrop
/// effect or mask): those images are the canvas's to reuse and recycle, so a scene that kept one
/// could later draw another layer's pixels under the old layer's key.
fn has_effects(doc: &Document, n: &Node) -> bool {
    let mut stack = vec![n.id];
    let mut seen = 0usize;
    while let Some(id) = stack.pop() {
        seen += 1;
        if seen > 4096 {
            return true;
        }
        let Some(node) = doc.node(id) else { continue };
        if node.is_mask {
            return true;
        }
        let image = match effects_of(doc, node) {
            Effects::None => false,
            // Drop shadows drawn from the outline are vello's own blurred rects: no image.
            Effects::Shadows(shape) => {
                let silhouette = figcraft_render::shadow::silhouette(doc, node);
                shape.shadows.iter().any(|s| !casts_from_outline(&silhouette, s))
            }
            _ => true,
        };
        if image {
            return true;
        }
        stack.extend(node.children.iter().copied());
    }
    false
}

/// Whether `n`'s pixels depend on the page under it: its own backdrop effect, or one nested in it.
fn reads_backdrop(doc: &Document, n: &Node) -> bool {
    (n.visible && backdrop_of(n).is_some()) || nests_backdrop(doc, n, 0)
}

/// Draw a backdrop node over its effect pixels (`image`, made from the page drawn under it).
pub fn draw_backdrop_into(
    doc: &Document,
    page: NodeId,
    view: &View,
    layers: &mut dyn Layers,
    id: NodeId,
    image: &Layer,
    out: &mut Scene,
) -> Result<(), Unsupported> {
    let Some(n) = doc.node(id) else { return Ok(()) };
    if !n.visible || n.opacity <= 0.0 {
        return Ok(());
    }
    if n.is_mask {
        return Err(Unsupported::Mask);
    }
    let Some(effect) = backdrop_of(n) else { return Err(Unsupported::Effect) };
    let mut b = Builder {
        doc,
        scene: Some(Scene::new()),
        layers,
        view: view.clone(),
        page,
        hidden: Vec::new(),
        no_strokes: Vec::new(),
        as_content: Vec::new(),
        blend_as_normal: Vec::new(),
    };
    // A blur's pixels are its padded layer (a rectangle, so the shape clips them); a pane's pixels
    // are its layer, with the shape's own coverage in the alpha.
    let place = match effect {
        Backdrop::Blur(_) => {
            let ((x, y), _) = blur_rect(doc, id, view).ok_or(Unsupported::Structure)?;
            ((x, y), true)
        }
        Backdrop::Glass(_) => {
            let (x0, y0, _, _) = b.layer_rect(n)?.ok_or(Unsupported::Structure)?;
            ((x0, y0), false)
        }
    };
    b.draw_backdrop_image(n, 0, image, place)?;
    if let Some(s) = b.scene.take() {
        out.append(&s, None);
    }
    Ok(())
}

/// A layer the GPU (or, while only checking a page, a stand-in) holds: its pixels for the scene to
/// draw, and the texture the GPU passes read.
#[derive(Clone)]
pub struct Layer {
    pub image: ImageData,
    pub(crate) texture: Option<wgpu::Texture>,
    pub size: (u32, u32),
}

impl Layer {
    /// A stand-in with no pixels: what the feature check hands the builder.
    fn blank(width: u32, height: u32) -> Self {
        Self { image: crate::canvas::blank_image(width, height), texture: None, size: (width, height) }
    }
}

/// Where a node's effect layer goes: the GPU renders it offscreen and blurs it without leaving the
/// device (see `crate::canvas::GpuLayers`). [`supported`] accepts such layers without drawing them.
pub trait Layers {
    /// A blurred `width` × `height` layer of `scene` (device pixels, transparent ground).
    fn blurred(&mut self, scene: &Scene, width: u32, height: u32, sigma: f64) -> Option<Layer>;
    /// What lies under a node — a whole page drawn into a `width` × `height` texture — blurred.
    fn backdrop(&mut self, scene: &Scene, width: u32, height: u32, sigma: f64) -> Option<Layer>;
    /// A glass pane's pixels from `scene` (the page under the node); its layer's top-left is `at`
    /// in the `view`-sized scene, and `pane.pad` around it is the backdrop it reads.
    fn glass(&mut self, scene: &Scene, view: (u32, u32), pane: &figcraft_render::glass::GlassPane, at: (i32, i32)) -> Option<Layer>;
    /// A backdrop node's effect pixels from `back`, the accumulated page (`view`-sized): the whole
    /// view blurred, or a glass pane's layer computed from a crop of it.
    fn backdrop_effect(&mut self, doc: &Document, id: NodeId, view: &View, back: &wgpu::Texture) -> Option<Layer>;
    /// `scene` drawn into a layer of its own, with no effect on it: a node's content (and its inner
    /// shadows), which its layer blur, grain and shadows then read.
    fn layer(&mut self, scene: &Scene, width: u32, height: u32, key: Option<(u64, u64)>) -> Option<Layer>;
    /// `layer` blurred by `sigma` (one node's layer blur, after whatever came before it).
    fn blur_layer(&mut self, layer: &Layer, sigma: f64, key: Option<(u64, u64)>) -> Option<Layer>;
    /// `layer` blurred progressively: `s0` at the layer's pixel `a`, `s1` at `b` (the renderer's
    /// blend of a few uniform blurs between them).
    fn progressive(&mut self, layer: &Layer, s0: f64, s1: f64, a: (f64, f64), b: (f64, f64), key: Option<(u64, u64)>) -> Option<Layer>;
    /// A shadow cast by `scene` (a silhouette, or a shape inverted for an inner shadow): its pixels
    /// `width` × `height` device pixels, blurred by `sigma` and tinted `color`. `key` lets a shadow
    /// that did not change be kept between frames.
    fn shadow(&mut self, scene: &Scene, width: u32, height: u32, sigma: f64, color: Color, key: Option<(u64, u64)>) -> Option<Layer>;
    /// A shadow cast by a node's own pixels: `scene`'s alpha grown (`grow > 0`) or shrunk by whole
    /// pixels, blurred by `sigma` and tinted `color`.
    fn shadow_of_pixels(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        grow: i32,
        sigma: f64,
        color: Color,
        key: Option<(u64, u64)>,
    ) -> Option<Layer>;
    /// A shadow cast by `layer`'s own pixels (a layer blur or grain before it, already in them).
    fn shadow_of_layer(&mut self, layer: &Layer, grow: i32, sigma: f64, color: Color, key: Option<(u64, u64)>) -> Option<Layer>;
    /// `layer`'s pixels, for the effects the CPU works out (grain's noise, the texture's scatter).
    fn layer_pixels(&mut self, layer: &Layer) -> Option<Pixmap>;
    /// `content` cut by `mask`'s luminance × alpha into one layer (a luminance mask; the CPU's
    /// `MaskType::Luminance`), kept under `key` when it was made before.
    fn luminance_masked(&mut self, _content: &Scene, _mask: &Layer, _width: u32, _height: u32, _key: Option<(u64, u64)>) -> Option<Layer> {
        None
    }
    /// A layer kept from an earlier frame under `key` (`None`: there is none, or keeping is off).
    fn kept_shadow(&mut self, _key: (u64, u64)) -> Option<Layer> {
        None
    }
    /// A page-level node's encoded scene from an earlier frame under `key` (its subtree, the view
    /// and the zoom): what a drag reuses instead of encoding the node again.
    fn cached_node(&mut self, _key: u64) -> Option<Arc<Scene>> {
        None
    }
    /// Keep `scene` as the encoded scene for `key` (the GPU canvas does; exports and the probe
    /// build every frame).
    fn keep_node(&mut self, _key: u64, _scene: Arc<Scene>) {}
}

/// Accepts effect layers without drawing them (the feature check).
struct Probe;

impl Layers for Probe {
    fn blurred(&mut self, _scene: &Scene, width: u32, height: u32, _sigma: f64) -> Option<Layer> {
        Some(Layer::blank(width, height))
    }
    fn backdrop(&mut self, scene: &Scene, width: u32, height: u32, sigma: f64) -> Option<Layer> {
        self.blurred(scene, width, height, sigma)
    }
    fn glass(&mut self, _scene: &Scene, _view: (u32, u32), pane: &figcraft_render::glass::GlassPane, _at: (i32, i32)) -> Option<Layer> {
        Some(Layer::blank(pane.w, pane.h))
    }
    fn backdrop_effect(&mut self, _doc: &Document, _id: NodeId, view: &View, _back: &wgpu::Texture) -> Option<Layer> {
        Some(Layer::blank(view.width, view.height))
    }
    fn layer(&mut self, _scene: &Scene, width: u32, height: u32, _key: Option<(u64, u64)>) -> Option<Layer> {
        Some(Layer::blank(width, height))
    }
    fn blur_layer(&mut self, layer: &Layer, _sigma: f64, _key: Option<(u64, u64)>) -> Option<Layer> {
        Some(Layer::blank(layer.size.0, layer.size.1))
    }
    fn progressive(&mut self, layer: &Layer, _s0: f64, _s1: f64, _a: (f64, f64), _b: (f64, f64), _key: Option<(u64, u64)>) -> Option<Layer> {
        Some(Layer::blank(layer.size.0, layer.size.1))
    }
    fn shadow(&mut self, _scene: &Scene, width: u32, height: u32, _sigma: f64, _color: Color, _key: Option<(u64, u64)>) -> Option<Layer> {
        Some(Layer::blank(width, height))
    }
    fn shadow_of_pixels(
        &mut self,
        _scene: &Scene,
        width: u32,
        height: u32,
        _grow: i32,
        _sigma: f64,
        _color: Color,
        _key: Option<(u64, u64)>,
    ) -> Option<Layer> {
        Some(Layer::blank(width, height))
    }
    fn shadow_of_layer(&mut self, layer: &Layer, _grow: i32, _sigma: f64, _color: Color, _key: Option<(u64, u64)>) -> Option<Layer> {
        Some(Layer::blank(layer.size.0, layer.size.1))
    }
    fn luminance_masked(&mut self, _content: &Scene, _mask: &Layer, width: u32, height: u32, _key: Option<(u64, u64)>) -> Option<Layer> {
        Some(Layer::blank(width, height))
    }
    fn layer_pixels(&mut self, layer: &Layer) -> Option<Pixmap> {
        Pixmap::new(layer.size.0, layer.size.1)
    }
}

/// Refuses effect layers: a build without a device.
pub struct NoLayers;

impl Layers for NoLayers {
    fn blurred(&mut self, _scene: &Scene, _width: u32, _height: u32, _sigma: f64) -> Option<Layer> {
        None
    }
    fn backdrop(&mut self, _scene: &Scene, _width: u32, _height: u32, _sigma: f64) -> Option<Layer> {
        None
    }
    fn glass(&mut self, _scene: &Scene, _view: (u32, u32), _pane: &figcraft_render::glass::GlassPane, _at: (i32, i32)) -> Option<Layer> {
        None
    }
    fn backdrop_effect(&mut self, _doc: &Document, _id: NodeId, _view: &View, _back: &wgpu::Texture) -> Option<Layer> {
        None
    }
    fn layer(&mut self, _scene: &Scene, _width: u32, _height: u32, _key: Option<(u64, u64)>) -> Option<Layer> {
        None
    }
    fn blur_layer(&mut self, _layer: &Layer, _sigma: f64, _key: Option<(u64, u64)>) -> Option<Layer> {
        None
    }
    fn progressive(&mut self, _layer: &Layer, _s0: f64, _s1: f64, _a: (f64, f64), _b: (f64, f64), _key: Option<(u64, u64)>) -> Option<Layer> {
        None
    }
    fn shadow(&mut self, _scene: &Scene, _width: u32, _height: u32, _sigma: f64, _color: Color, _key: Option<(u64, u64)>) -> Option<Layer> {
        None
    }
    fn shadow_of_pixels(
        &mut self,
        _scene: &Scene,
        _width: u32,
        _height: u32,
        _grow: i32,
        _sigma: f64,
        _color: Color,
        _key: Option<(u64, u64)>,
    ) -> Option<Layer> {
        None
    }
    fn shadow_of_layer(&mut self, _layer: &Layer, _grow: i32, _sigma: f64, _color: Color, _key: Option<(u64, u64)>) -> Option<Layer> {
        None
    }
    fn layer_pixels(&mut self, _layer: &Layer) -> Option<Pixmap> {
        None
    }
}

/// Whether `page` can be drawn on the GPU at all: `Ok` when every visible layer is in the
/// builder's feature set, else the first reason it isn't.
pub fn supported(doc: &Document, page: NodeId) -> Result<(), Unsupported> {
    let mut probe = Probe;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: 1, height: 1 };
    let mut b = Builder {
        doc,
        scene: None,
        layers: &mut probe,
        view,
        page,
        hidden: Vec::new(),
        no_strokes: Vec::new(),
        as_content: Vec::new(),
        blend_as_normal: Vec::new(),
    };
    b.children(page, Affine::IDENTITY, 0)
}

/// Draw page `page` of `doc` as seen through `view` into a vello scene (no effect layers).
pub fn build(doc: &Document, page: NodeId, view: &View) -> Result<Scene, Unsupported> {
    build_with(doc, page, view, &mut NoLayers)
}

/// Draw page `page` of `doc` as seen through `view` into a vello scene, drawing effect layers
/// through `layers` (the GPU canvas): a page-level node at a time, each reused from `layers` when
/// it did not change.
pub fn build_with(doc: &Document, page: NodeId, view: &View, layers: &mut dyn Layers) -> Result<Scene, Unsupported> {
    let mut out = Scene::new();
    let Some(p) = doc.node(page) else { return Ok(out) };
    for c in &p.children {
        draw_top(doc, page, view, layers, *c, &mut out, false)?;
    }
    Ok(out)
}

struct Builder<'a> {
    doc: &'a Document,
    /// `None` while only checking whether the page is supported.
    scene: Option<Scene>,
    layers: &'a mut dyn Layers,
    view: View,
    /// The page drawn (backdrop effects render what lies under a node).
    page: NodeId,
    /// Layers left out (painted after a backdrop node: see [`figcraft_render::backdrop::above`]).
    hidden: Vec<NodeId>,
    /// Layers whose strokes are left out (they are painted after their children's).
    no_strokes: Vec<NodeId>,
    /// Mask nodes drawn as ordinary content (their coverage, for the siblings they clip).
    as_content: Vec<NodeId>,
    /// Nodes whose plus-darker blend the canvas applies itself (a step of their own): drawn here
    /// without it.
    blend_as_normal: Vec<NodeId>,
}

/// What a node's visible effects need of the GPU, if anything.
enum Effects {
    None,
    /// One background blur (uniform): its radius in document units.
    BackgroundBlur(f64),
    /// One glass pane.
    Glass(figcraft_doc::Glass),
    /// Drop shadows a node casts straight into the scene: its silhouette is one rounded rect, so
    /// each shadow is vello's blurred rounded rect under the node, with no layer.
    Shadows(ShadowShape),
    /// Inner shadows only: everything outside the shape at the shadow's offset, blurred and
    /// clipped to the shape (drawn between the node's fills and its children).
    InnerShadows,
    /// A layer of its own: a layer blur, grain, texture or drop shadows (any mix, with inner
    /// shadows too), in the CPU's order.
    Layer,
    /// Anything else (a shadow beside a backdrop effect).
    Other,
}

/// A node whose drop shadows are drawn straight into the scene (no layer).
struct ShadowShape {
    /// The corner radius of the node's outline.
    radius: f64,
    /// How far its paints reach past the outline: a centred stroke's outer half, all of an outside
    /// one — the silhouette's own growth, in the node's units.
    grow: f64,
    /// See [`fill_alpha`]: what the shadow is seen through, and so what it is made of.
    alpha: f64,
    shadows: Vec<figcraft_doc::Shadow>,
}

/// Whether the node's own outline is a rectangle with corners: a shape or a frame, component,
/// instance or section.
fn is_rect_kind(k: &NodeKind) -> bool {
    matches!(
        k,
        NodeKind::Rectangle
            | NodeKind::Frame
            | NodeKind::Component { .. }
            | NodeKind::ComponentSet { .. }
            | NodeKind::Instance { .. }
            | NodeKind::Section { .. }
    )
}

/// The alpha the node's own fills paint the whole shape with, when it is one value (a shadow is
/// cast by the layer's pixels, so a translucent card casts one just as faint); `None` when the
/// fills vary across it or composite in a way one alpha can't say.
fn fill_alpha(n: &Node) -> Option<f64> {
    let mut clear = 1.0f64;
    let mut any = false;
    for p in n.fills.iter().filter(|p| p.visible && p.opacity > 0.0) {
        if p.blend != BlendMode::Normal {
            return None;
        }
        let op = f64::from(p.opacity.clamp(0.0, 1.0));
        let alpha = match &p.kind {
            PaintKind::Solid { color } => f64::from(color.a) * op,
            PaintKind::Gradient(g) => {
                let first = f64::from(g.stops.first()?.color.a) * op;
                if g.stops.iter().any(|s| (f64::from(s.color.a) * op - first).abs() > 1e-6) {
                    return None;
                }
                first
            }
            PaintKind::Image(_) | PaintKind::Pattern(_) => return None,
        };
        clear *= 1.0 - alpha.clamp(0.0, 1.0);
        any = true;
    }
    any.then_some(1.0 - clear)
}

/// Whether `p` paints opaque coverage (its blend mode aside: what it covers hides a shadow under
/// the layer exactly there).
fn paint_opaque(p: &Paint) -> bool {
    p.visible
        && p.opacity >= 1.0
        && p.blend == BlendMode::Normal
        && match &p.kind {
            PaintKind::Solid { color } => color.a >= 1.0,
            PaintKind::Gradient(g) => g.stops.iter().all(|s| s.color.a >= 1.0),
            PaintKind::Image(_) | PaintKind::Pattern(_) => false,
        }
}

/// The rounded rect a node's drop shadows cast from, when every shadow can be drawn straight into
/// the scene; `None` leaves the node (and the page) to the CPU.
///
/// The CPU casts a shadow from the layer's pixels (or, when it spreads, from the paints' outline)
/// and hides it behind the paints' *cover*: a fill or stroke hides it whether or not it is
/// translucent. A shadow drawn under an opaque node is hidden exactly there; a translucent one
/// hides it with a clip outside its own outline, and casts it just as faint as it paints. So the
/// fills must paint the shape with one alpha (an opaque stroke may surround them), the node's
/// opacity is 1 and its blend normal. Children may paint over the shadow wherever they are; they
/// are allowed only in a clipping frame, where they cannot reach past the shape.
fn shadow_shape(doc: &Document, n: &Node) -> Option<ShadowShape> {
    if !is_rect_kind(&n.kind) || n.corners.smoothing != 0.0 {
        return None;
    }
    if n.opacity < 1.0 || !matches!(n.blend, BlendMode::PassThrough | BlendMode::Normal) {
        return None;
    }
    let radius = n.corners.uniform()?.max(0.0);
    let alpha = fill_alpha(n)?;
    let stroked = n.has_visible_stroke();
    // A translucent fill beside a stroke casts two different shadows (the band is opaque).
    if stroked && alpha < 1.0 {
        return None;
    }
    let grow = if stroked {
        let stroke = &n.stroke;
        let weight = if stroke.weight.is_finite() { stroke.weight.max(0.0) } else { 0.0 };
        // A band that isn't the plain outline (dashes, a width profile, per-side widths, a brush)
        // is not a rounded rect; a translucent band would let the shadow show through it.
        if stroke.sides.is_some() || !stroke.dashes.is_empty() || stroke.width_profile.is_some() || stroke.brush.is_some() {
            return None;
        }
        if !n.strokes.iter().filter(|p| p.visible).all(paint_opaque) {
            return None;
        }
        match stroke.align {
            figcraft_doc::StrokeAlign::Inside => 0.0,
            figcraft_doc::StrokeAlign::Center => weight / 2.0,
            figcraft_doc::StrokeAlign::Outside => weight,
        }
    } else {
        0.0
    };
    let kids = n.children.iter().any(|c| doc.node(*c).is_some_and(|c| c.visible && c.opacity > 0.0));
    if kids && !(n.kind.is_frame_like() && n.clip) {
        return None;
    }
    let mut shadows = Vec::new();
    for e in n.effects.iter().filter(|e| e.visible) {
        let figcraft_doc::EffectKind::DropShadow(s) = &e.kind else { return None };
        if !(s.offset.0.is_finite() && s.offset.1.is_finite() && s.radius.is_finite() && s.spread.is_finite()) {
            return None;
        }
        if matches!(s.blend, BlendMode::PlusLighter | BlendMode::PlusDarker) {
            return None;
        }
        shadows.push(s.clone());
    }
    (!shadows.is_empty()).then_some(ShadowShape { radius, grow, alpha, shadows })
}

fn effects_of(doc: &Document, n: &Node) -> Effects {
    if let Some(b) = backdrop_of(n) {
        return match b {
            Backdrop::Blur(radius) => Effects::BackgroundBlur(radius),
            Backdrop::Glass(g) => Effects::Glass(g),
        };
    }
    let visible = || n.effects.iter().filter(|e| e.visible);
    if visible().next().is_none() {
        return Effects::None;
    }
    let only = |wanted: fn(&figcraft_doc::EffectKind) -> bool| visible().all(|e| wanted(&e.kind));
    // Drop shadows on one rounded rect: straight into the scene, no layer.
    if only(|k| matches!(k, figcraft_doc::EffectKind::DropShadow(_))) {
        return match shadow_shape(doc, n) {
            Some(shape) => Effects::Shadows(shape),
            None => Effects::Layer,
        };
    }
    if only(|k| matches!(k, figcraft_doc::EffectKind::InnerShadow(_))) {
        // A group's is cast from its pixels: it needs the node's layer, like the other effects.
        return if matches!(n.kind, NodeKind::Group | NodeKind::Page { .. }) { Effects::Layer } else { Effects::InnerShadows };
    }
    // One layer of its own, in the CPU's order: inner shadows, the layer blur, grain and texture,
    // then the drop shadows cast from that result.
    let known = |k: &figcraft_doc::EffectKind| {
        matches!(
            k,
            figcraft_doc::EffectKind::DropShadow(_)
                | figcraft_doc::EffectKind::InnerShadow(_)
                | figcraft_doc::EffectKind::Noise(_)
                | figcraft_doc::EffectKind::LayerBlur(_)
                | figcraft_doc::EffectKind::Texture(_)
        )
    };
    if visible().all(|e| known(&e.kind)) {
        return Effects::Layer;
    }
    Effects::Other
}

impl Builder<'_> {
    fn children(&mut self, id: NodeId, xf: Affine, depth: usize) -> Result<(), Unsupported> {
        let Some(n) = self.doc.node(id) else { return Ok(()) };
        let reverse = n.layout.as_ref().is_some_and(|l| l.reverse_z);
        let doc = self.doc;
        let nodes: Vec<Option<&Node>> = n.children.iter().map(|c| doc.node(*c)).collect();
        for run in Document::paint_runs_of(&nodes, reverse) {
            let Some(Some(cn)) = nodes.get(run.start) else { continue };
            if cn.is_mask && cn.visible {
                // Everything above the mask (until the next mask) is clipped by its coverage.
                let content = n.children.get(run.start + 1..run.end).unwrap_or(&[]);
                self.masked(cn, content, xf, depth)?;
            } else {
                self.node(n.children[run.start], xf, depth)?;
            }
        }
        Ok(())
    }

    /// A mask and the siblings it clips (until the next mask), in a layer of their own and cut by
    /// the mask's coverage — the renderer's `draw_masked`. An alpha mask is the node's own paint;
    /// a vector mask is its outline, solid; a luminance mask is its paint's luma × alpha.
    fn masked(&mut self, mask: &Node, content: &[NodeId], parent_xf: Affine, depth: usize) -> Result<(), Unsupported> {
        let luma = mask.mask_type == figcraft_doc::MaskType::Luminance;
        // What the run covers, in device pixels, clipped to what is on screen.
        let mut rect: Option<KRect> = None;
        for id in content {
            let Some(n) = self.doc.node(*id) else { continue };
            if !n.visible || n.opacity <= 0.0 {
                continue;
            }
            let Some((x, y, w, h)) = self.layer_rect(n)? else { continue };
            let r = KRect::new(x, y, x + f64::from(w), y + f64::from(h));
            rect = Some(rect.map_or(r, |a| a.union(r)));
        }
        let view = self.view.clone();
        let screen = KRect::new(0.0, 0.0, f64::from(view.width), f64::from(view.height));
        let Some(rect) = rect.map(|r| r.intersect(screen)) else { return Ok(()) };
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return Ok(());
        }
        let (x0, y0) = (rect.x0.floor(), rect.y0.floor());
        let (w, h) = ((rect.x1.ceil() - x0).min(f64::from(f32::MAX)) as u32, (rect.y1.ceil() - y0) as u32);
        if w == 0 || h == 0 {
            return Ok(());
        }
        let xf = view.transform();
        let key = self.shadow_key(mask, u64::MAX - 2, xf * Affine::translate((x0, y0)));
        let sampler = ImageSampler { x_extend: Extend::Pad, y_extend: Extend::Pad, quality: ImageQuality::Low, alpha: 1.0 };
        if let Some(hit) = self.layers.kept_shadow(key) {
            if let Some(s) = &mut self.scene {
                s.draw_image(ImageBrush { image: &hit.image, sampler }, KAffine::translate((x0, y0)));
            }
            return Ok(());
        }
        // The layer's own pixels are a view of their own: every effect inside lands in it.
        let sub = View { origin: (view.origin.0 + x0 / view.scale, view.origin.1 + y0 / view.scale), scale: view.scale, width: w, height: h };
        // `parent_xf` is in page pixels: the layer's own grid starts at `(x0, y0)`.
        let at = Affine::translate((-x0, -y0)) * parent_xf;
        let coverage = if mask.mask_type == figcraft_doc::MaskType::Vector {
            let mut scene = Scene::new();
            if let Some((outline, rule)) = figcraft_render::subtree_outline(self.doc, mask, depth + 1) {
                let white = PColor::new([1.0, 1.0, 1.0, 1.0]);
                scene.fill(fill_rule(rule), k_affine(at * mask.transform), &Brush::Solid(white), None, &outline);
            }
            scene
        } else {
            self.as_content.push(mask.id);
            let built = self.sub_scene(sub.clone(), |b| b.node(mask.id, at, depth + 1));
            self.as_content.pop();
            built?
        };
        let content_scene = self.sub_scene(sub, |b| {
            for id in content {
                b.node(*id, at, depth + 1)?;
            }
            Ok(())
        })?;
        let Some(coverage_layer) = self.layers.layer(&coverage, w, h, None) else { return Err(Unsupported::Effect) };
        // A luminance mask multiplies by the mask's luma × alpha in one pass; an alpha mask keeps
        // the siblings only where the mask's alpha covers (`SrcIn`: source × destination's alpha).
        let out = if luma {
            self.layers.luminance_masked(&content_scene, &coverage_layer, w, h, Some(key))
        } else {
            let Some(content_layer) = self.layers.layer(&content_scene, w, h, None) else { return Err(Unsupported::Effect) };
            let mut scene = Scene::new();
            scene.draw_image(ImageBrush { image: &coverage_layer.image, sampler }, KAffine::IDENTITY);
            let target = KRect::new(0.0, 0.0, f64::from(w), f64::from(h));
            scene.push_layer(Fill::NonZero, Compose::SrcIn, 1.0, KAffine::IDENTITY, &target);
            scene.draw_image(ImageBrush { image: &content_layer.image, sampler }, KAffine::IDENTITY);
            scene.pop_layer();
            self.layers.layer(&scene, w, h, Some(key))
        };
        let Some(out) = out else { return Err(Unsupported::Effect) };
        if let Some(s) = &mut self.scene {
            s.draw_image(ImageBrush { image: &out.image, sampler }, KAffine::translate((x0, y0)));
        }
        Ok(())
    }

    /// What `f` draws through `view`, into a scene of its own.
    fn sub_scene<F>(&mut self, view: View, f: F) -> Result<Scene, Unsupported>
    where
        F: FnOnce(&mut Builder<'_>) -> Result<(), Unsupported>,
    {
        let mut b = Builder {
            doc: self.doc,
            scene: Some(Scene::new()),
            layers: &mut *self.layers,
            view,
            page: self.page,
            hidden: self.hidden.clone(),
            no_strokes: self.no_strokes.clone(),
            as_content: self.as_content.clone(),
            blend_as_normal: self.blend_as_normal.clone(),
        };
        f(&mut b)?;
        Ok(b.scene.unwrap_or_default())
    }

    fn node(&mut self, id: NodeId, parent_xf: Affine, depth: usize) -> Result<(), Unsupported> {
        if depth > MAX_DEPTH {
            return Err(Unsupported::Structure);
        }
        if self.hidden.contains(&id) {
            return Ok(());
        }
        let Some(n) = self.doc.node(id) else { return Ok(()) };
        if !n.visible || n.opacity <= 0.0 {
            return Ok(());
        }
        if n.is_mask && !self.as_content.contains(&id) {
            return Err(Unsupported::Mask);
        }
        match effects_of(self.doc, n) {
            Effects::None => {}
            Effects::BackgroundBlur(radius) => return self.backdrop_layer(n, depth, radius),
            Effects::Glass(glass) => return self.glass_layer(n, depth, &glass),
            Effects::Shadows(shape) => {
                let xf = parent_xf * n.transform;
                self.draw_shadows(n, &shape, xf);
            }
            Effects::Layer => return self.layer_pipeline(n, depth),
            // Drawn inside the content, between the fills and the children.
            Effects::InnerShadows => {}
            Effects::Other => return Err(Unsupported::Effect),
        }
        // vello has no plus-darker composition: the CPU draws it, or a step of its own does.
        if n.blend == BlendMode::PlusDarker && !self.blend_as_normal.contains(&n.id) {
            return Err(Unsupported::Structure);
        }
        let xf = parent_xf * n.transform;
        let isolated = n.opacity < 1.0 || !matches!(n.blend, BlendMode::PassThrough | BlendMode::Normal);
        if isolated {
            self.push_layer(n);
        }
        self.content(n, xf, depth)?;
        if isolated && let Some(s) = &mut self.scene {
            s.pop_layer();
        }
        Ok(())
    }

    /// Drop shadows drawn without a layer: vello's blurred rounded rect is the card's own
    /// silhouette (grown by its stroke and the shadow's spread) offset and blurred, and the card
    /// paints over it — it hides the shadow exactly where the CPU's cover mask does, because its
    /// paints are opaque over that area.
    fn draw_shadows(&mut self, n: &Node, shape: &ShadowShape, xf: Affine) {
        let Some(scene) = &mut self.scene else { return };
        let rect = KRect::new(0.0, 0.0, n.width, n.height);
        let scale = xf.determinant().abs().sqrt();
        // A translucent card hides its shadow where its paints cover it (the CPU's cover mask) but
        // lets it through elsewhere: the shadow is clipped to the outside of the card's outline.
        let outside = if shape.alpha < 1.0 {
            let mut hole = KRect::new(-1e5, -1e5, 1e5, 1e5).to_path(0.0);
            let inner = figcraft_geom::rounded_rect(n.width, n.height, n.corners.radii, 0.0);
            hole.extend(inner.elements().iter().copied());
            Some(hole)
        } else {
            None
        };
        for s in &shape.shadows {
            // A shadow that spreads is cast from the outline at its exact offset; one that does
            // not is cast from the layer's pixels, which the CPU moves by whole device pixels.
            let snap = |v: f64| if s.spread == 0.0 && scale.is_finite() && scale > 0.0 { (v * scale).round() / scale } else { v };
            let grow = shape.grow + s.spread;
            let rect = rect.inflate(grow, grow);
            if rect.width() <= 0.0 || rect.height() <= 0.0 {
                continue;
            }
            let radius = (shape.radius + grow).max(0.0);
            let c = s.color.sanitized();
            // A spreading shadow is cast from the paints' outline, which is opaque coverage: the CPU
            // reads the fills' alpha only when the shadow comes from the layer's pixels.
            let alpha = f64::from(c.a) * if s.spread != 0.0 { 1.0 } else { shape.alpha };
            if !alpha.is_finite() || alpha <= 0.0 {
                continue;
            }
            let at = KAffine::translate((snap(s.offset.0), snap(s.offset.1))) * k_affine(xf);
            // A shadow with a blend mode of its own composites as a layer; the usual one is drawn
            // straight into the scene.
            let blended = s.blend != BlendMode::Normal && s.blend != BlendMode::PassThrough;
            if blended {
                let target = KRect::new(0.0, 0.0, 32_768.0, 32_768.0);
                scene.push_layer(Fill::NonZero, blend(s.blend), 1.0, KAffine::IDENTITY, &target);
            }
            // A shadow that shows behind the layer is not hidden by the paints at all.
            let clipped = outside.is_some() && !s.show_behind;
            if let Some(hole) = outside.as_ref().filter(|_| clipped) {
                scene.push_clip_layer(Fill::EvenOdd, k_affine(xf), hole);
            }
            // The radius and the deviation are in the shape's own units (the transform scales
            // them, and the blur with them).
            scene.draw_blurred_rounded_rect(at, rect, PColor::new([c.r, c.g, c.b, alpha as f32]), radius, (s.radius / 2.0).max(0.0));
            if clipped {
                scene.pop_layer();
            }
            if blended {
                scene.pop_layer();
            }
        }
    }

    /// The page as it is under node `id`: the background, then every layer painted before it (the
    /// node and everything after it left out).
    fn backdrop_of(&mut self, id: NodeId) -> Result<Scene, Unsupported> {
        let view = self.view.clone();
        let page = self.page;
        let above = figcraft_render::backdrop::above(self.doc, id, &[page]);
        let mut below = Builder {
            doc: self.doc,
            scene: Some(Scene::new()),
            layers: &mut *self.layers,
            view: view.clone(),
            page,
            hidden: above.hidden,
            no_strokes: above.strokes,
            as_content: Vec::new(),
            blend_as_normal: Vec::new(),
        };
        if let Some(s) = &mut below.scene {
            let bg = background(self.doc, page);
            s.fill(Fill::NonZero, KAffine::IDENTITY, PColor::new([bg.r, bg.g, bg.b, bg.a]), None, &view_rect(&view));
        }
        if let Some(p) = self.doc.node(page) {
            for c in &p.children {
                below.node(*c, view.transform(), 0)?;
            }
        }
        below.scene.take().ok_or(Unsupported::Structure)
    }

    /// A node with one background blur: everything painted under it (the page without it or
    /// anything after it) is drawn and blurred on the GPU, shown through its shape, and its own
    /// content goes over that.
    fn backdrop_layer(&mut self, n: &Node, depth: usize, radius: f64) -> Result<(), Unsupported> {
        if n.blend == BlendMode::PlusDarker {
            return Err(Unsupported::Structure);
        }
        let view = self.view.clone();
        let back = self.backdrop_of(n.id)?;
        let sigma = radius / 2.0 * view.scale;
        let Some(image) = self.layers.backdrop(&back, view.width, view.height, sigma) else { return Err(Unsupported::Effect) };
        self.draw_backdrop_image(n, depth, &image, ((0.0, 0.0), true))
    }

    /// A node with one glass effect: its layer's pixels come from the page under it, refracted by
    /// the GPU's glass shader (the CPU's geometry), and its own content goes over them.
    fn glass_layer(&mut self, n: &Node, depth: usize, glass: &figcraft_doc::Glass) -> Result<(), Unsupported> {
        if n.blend == BlendMode::PlusDarker {
            return Err(Unsupported::Structure);
        }
        let view = self.view.clone();
        let Some((x0, y0, w, h)) = self.layer_rect(n)? else { return Ok(()) };
        let parent = n.parent.map(|p| self.doc.world_transform(p)).unwrap_or(Affine::IDENTITY);
        let xf = view.transform() * parent * n.transform;
        let Some(shape) = figcraft_render::backdrop::outline(self.doc, n) else { return Err(Unsupported::Structure) };
        let layer_xf = Affine::translate((-x0, -y0)) * xf;
        let mut pane = figcraft_render::glass::pane(glass, &shape, w, h, layer_xf, view.scale).ok_or(Unsupported::Effect)?;
        pane.pad = figcraft_render::backdrop::pad(n, view.scale);
        let back = self.backdrop_of(n.id)?;
        let Some(image) = self.layers.glass(&back, (view.width, view.height), &pane, (x0 as i32, y0 as i32)) else {
            return Err(Unsupported::Effect);
        };
        self.draw_backdrop_image(n, depth, &image, ((x0, y0), false))
    }

    /// The device rectangle of `n`'s effect layer, in whole pixels (`None` past the layer cap).
    fn layer_rect(&self, n: &Node) -> Result<Option<(f64, f64, u32, u32)>, Unsupported> {
        layer_rect_impl(self.doc, n, &self.view)
    }

    /// [`Self::layer_rect`] at the layer's exact device corner: where its content's own transform
    /// starts, so a subpixel move leaves the content and moves the layer.
    fn layer_rect_exact(&self, n: &Node) -> Result<Option<(f64, f64, u32, u32)>, Unsupported> {
        layer_rect_exact_impl(self.doc, n, &self.view)
    }

    /// `n`'s backdrop effect pixels: its layer texture with its top-left at `at`, clipped to the
    /// node's shape when `clip` (a blurred layer is a rectangle; a pane carries its own coverage);
    /// then the node's content over them.
    fn draw_backdrop_image(&mut self, n: &Node, depth: usize, image: &Layer, ((x, y), clip): ((f64, f64), bool)) -> Result<(), Unsupported> {
        let view = self.view.clone();
        let parent = n.parent.map(|p| self.doc.world_transform(p)).unwrap_or(Affine::IDENTITY);
        let xf = view.transform() * parent * n.transform;
        let isolated = n.opacity < 1.0 || !matches!(n.blend, BlendMode::PassThrough | BlendMode::Normal);
        if isolated {
            self.push_layer(n);
        }
        let sampler = ImageSampler { x_extend: Extend::Pad, y_extend: Extend::Pad, quality: ImageQuality::Low, alpha: 1.0 };
        let image = &image.image;
        if clip {
            if let Some((path, rule)) = node_outline(self.doc, n)
                && let Some(s) = &mut self.scene
            {
                s.push_clip_layer(fill_rule(rule), k_affine(xf), &path);
                s.draw_image(ImageBrush { image, sampler }, KAffine::translate((x, y)));
                s.pop_layer();
            }
        } else if let Some(s) = &mut self.scene {
            s.draw_image(ImageBrush { image, sampler }, KAffine::translate((x, y)));
        }
        self.content(n, xf, depth)?;
        if isolated && let Some(s) = &mut self.scene {
            s.pop_layer();
        }
        Ok(())
    }

    /// The node's content alone, drawn into its own `width` × `height` layer (what a blur, a
    /// shadow or a backdrop effect reads), with the layer's top-left at `(x0, y0)`.
    fn content_scene(&mut self, n: &Node, (x0, y0, w, h): (f64, f64, u32, u32), depth: usize) -> Result<Scene, Unsupported> {
        let view = self.view.clone();
        let scale = view.scale.max(1e-9);
        let layer = View { origin: (view.origin.0 + x0 / scale, view.origin.1 + y0 / scale), scale: view.scale, width: w, height: h };
        let parent = n.parent.map(|p| self.doc.world_transform(p)).unwrap_or(Affine::IDENTITY);
        let mut sub = Builder {
            doc: self.doc,
            scene: Some(Scene::new()),
            layers: &mut *self.layers,
            view: layer.clone(),
            page: self.page,
            hidden: self.hidden.clone(),
            no_strokes: self.no_strokes.clone(),
            as_content: self.as_content.clone(),
            blend_as_normal: self.blend_as_normal.clone(),
        };
        // The content only: the node's opacity and blend mode apply to the result, not inside it.
        sub.content(n, layer.transform() * parent * n.transform, depth)?;
        sub.scene.take().ok_or(Unsupported::Structure)
    }

    /// A node's own layer and the effects that read it, in the CPU's order: its content (and inner
    /// shadows) → its layer blur and grain → the drop shadows cast from that result → the layer
    /// over them. The shadows the rounded-rect fast path draws straight into the scene still are.
    fn layer_pipeline(&mut self, n: &Node, depth: usize) -> Result<(), Unsupported> {
        if n.blend == BlendMode::PlusDarker && !self.blend_as_normal.contains(&n.id) {
            return Err(Unsupported::Structure);
        }
        let Some((x0f, y0f, w, h)) = self.layer_rect_exact(n)? else { return Ok(()) };
        // A layer whose pixels this frame *makes* (a blur, noise, texture) is drawn from its exact
        // corner and composited on whole pixels: its content is laid down, not sampled, wherever
        // it lands, and it is up to half a device pixel off the vector's place — which is what lets
        // a drag reuse it instead of blurring it again (`layer_rect_exact_impl`). Everything else
        // (a group, a card under a shadow) stays on the renderer's floored grid, where its edges
        // land exactly where the CPU's do.
        let reads = n.effects.iter().any(|e| {
            e.visible
                && matches!(
                    e.kind,
                    figcraft_doc::EffectKind::LayerBlur(_) | figcraft_doc::EffectKind::Noise(_) | figcraft_doc::EffectKind::Texture(_)
                )
        });
        let (bx0, by0) = if reads { (x0f, y0f) } else { (x0f.floor(), y0f.floor()) };
        let (x0, y0) = (bx0.round(), by0.round());
        // What is cut against the node's own pixels (a shadow's hole) follows the layer's grid.
        let snap = Affine::translate((x0 - bx0, y0 - by0));
        let view = self.view.clone();
        let parent = n.parent.map(|p| self.doc.world_transform(p)).unwrap_or(Affine::IDENTITY);
        let xf = view.transform() * parent * n.transform;
        // From the layer's corner: the content of a made layer keeps its place when the node moves
        // by part of a device pixel (`layer_rect_exact_impl`).
        let layer_xf = Affine::translate((-bx0, -by0)) * xf;
        let scale = view.scale;
        let effects: Vec<&figcraft_doc::Effect> = n.effects.iter().filter(|e| e.visible).collect();
        let shadows: Vec<figcraft_doc::Shadow> = effects
            .iter()
            .filter_map(|e| match &e.kind {
                figcraft_doc::EffectKind::DropShadow(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        for s in &shadows {
            if !(s.offset.0.is_finite() && s.offset.1.is_finite() && s.radius.is_finite() && s.spread.is_finite()) {
                return Err(Unsupported::Effect);
            }
        }
        let reads_pixels: Vec<&figcraft_doc::Effect> = effects
            .iter()
            .copied()
            .filter(|e| {
                matches!(e.kind, figcraft_doc::EffectKind::LayerBlur(_) | figcraft_doc::EffectKind::Noise(_) | figcraft_doc::EffectKind::Texture(_))
            })
            .collect();
        // A group has no outline of its own: its inner shadows are cast from around what its
        // children draw together (the renderer's own `inner_shadow_of`).
        let group_inner: Vec<figcraft_doc::Shadow> = if matches!(n.kind, NodeKind::Group) {
            effects
                .iter()
                .filter_map(|e| match &e.kind {
                    figcraft_doc::EffectKind::InnerShadow(s) => Some(s.clone()),
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        for s in &group_inner {
            if !(s.offset.0.is_finite() && s.offset.1.is_finite() && s.radius.is_finite() && s.spread.is_finite()) {
                return Err(Unsupported::Effect);
            }
        }
        let silhouette = figcraft_render::shadow::silhouette(self.doc, n);
        let outside = figcraft_render::shadow::cover(self.doc, n).map(|c| outside(&c));
        let pixel_shadows = shadows.iter().any(|s| !casts_from_outline(&silhouette, s));
        let bounds = KRect::new(x0, y0, x0 + f64::from(w), y0 + f64::from(h));
        // The node's own pixels: its content with its inner shadows, then its layer effects.
        let mut content: Option<Layer> = None;
        if pixel_shadows || !reads_pixels.is_empty() || !group_inner.is_empty() {
            // A layer's pixels are drawn in the layer's own space (its top-left at its own
            // origin): moving the node moves the layer's place, not what it holds, so the key is
            // the layer's own transform — a drag reuses the blurred layer instead of blurring it
            // again.
            let key = self.shadow_key(n, u64::MAX, layer_xf);
            content = self.layers.kept_shadow(key);
            if content.is_none() {
                let scene = self.content_scene(n, (bx0, by0, w, h), depth)?;
                let mut made = self.layers.layer(&scene, w, h, None).ok_or(Unsupported::Effect)?;
                for s in &group_inner {
                    made = self.group_inner_shadow(&made, s, scale)?;
                }
                for (i, e) in reads_pixels.iter().enumerate() {
                    // The last stage keeps its result: the layer is a function of the node.
                    let keep = (i + 1 == reads_pixels.len()).then_some(key);
                    made = match &e.kind {
                        figcraft_doc::EffectKind::LayerBlur(b) => {
                            let b = b.sanitized();
                            let (s0, s1) = (b.start_radius / 2.0 * scale, b.radius / 2.0 * scale);
                            if b.is_progressive() {
                                let at = |u: (f64, f64)| {
                                    let p = layer_xf * Point::new(u.0 * n.width, u.1 * n.height);
                                    (p.x, p.y)
                                };
                                self.layers.progressive(&made, s0, s1, at(b.start), at(b.end), keep).ok_or(Unsupported::Effect)?
                            } else {
                                self.layers.blur_layer(&made, s1, keep).ok_or(Unsupported::Effect)?
                            }
                        }
                        figcraft_doc::EffectKind::Noise(nz) => self.grain_layer(&made, nz, layer_xf, n.id.0 as u32, keep)?,
                        figcraft_doc::EffectKind::Texture(t) => self.texture_layer(&made, t, layer_xf, n.id.0 as u32, keep)?,
                        _ => return Err(Unsupported::Effect),
                    };
                }
                content = Some(made);
            }
        }
        for (index, s) in shadows.iter().enumerate() {
            let (dx, dy) = (s.offset.0 * scale, s.offset.1 * scale);
            let spread = s.spread;
            // The shadow is drawn at the node's opacity (as the CPU composites it), so its colour's
            // alpha carries it.
            let mut color = s.color.sanitized();
            color.a *= n.opacity.clamp(0.0, 1.0);
            if color.a <= 0.0 {
                continue;
            }
            let sigma = s.radius / 2.0 * scale;
            // The shadow's pixels are built in the layer's own space too: the node's place on the
            // page is its separate placement, so a drag keeps the cast shadow.
            let key = self.shadow_key(n, index as u64, layer_xf);
            let (image, at) = match self.layers.kept_shadow(key) {
                Some(image) => (Some(image), shadow_place(&silhouette, spread, (x0, y0), (dx, dy))),
                None => {
                    if casts_from_outline(&silhouette, s) {
                        let Some(sil) = &silhouette else { return Err(Unsupported::Effect) };
                        let mut scene = Scene::new();
                        silhouette_scene(&mut scene, sil, spread, layer_xf, (dx, dy), KRect::new(0.0, 0.0, f64::from(w), f64::from(h)));
                        (self.layers.shadow(&scene, w, h, sigma, color, Some(key)), (x0, y0))
                    } else {
                        let Some(layer) = content.as_ref() else { return Err(Unsupported::Structure) };
                        let grow = whole_px(spread * scale);
                        (
                            self.layers.shadow_of_layer(layer, grow, sigma, color, Some(key)),
                            (x0 + f64::from(whole_px(dx)), y0 + f64::from(whole_px(dy))),
                        )
                    }
                }
            };
            let Some(image) = image else { return Err(Unsupported::Effect) };
            // A shadow that hides behind the paints is cut where they cover (the CPU masks it by
            // its cover); one that shows behind is not.
            let hole = if s.show_behind { None } else { outside.as_ref() };
            self.draw_shadow_image(&image, at, Some(&bounds), hole, s.blend, snap * xf);
        }
        // The node over its shadows: the layer the effects made, or its content drawn straight
        // into the scene when nothing had to read it.
        let isolated = n.opacity < 1.0 || !matches!(n.blend, BlendMode::PassThrough | BlendMode::Normal);
        if isolated {
            self.push_layer(n);
        }
        match &content {
            Some(layer) => {
                let sampler = ImageSampler { x_extend: Extend::Pad, y_extend: Extend::Pad, quality: ImageQuality::Low, alpha: 1.0 };
                if let Some(s) = &mut self.scene {
                    s.draw_image(ImageBrush { image: &layer.image, sampler }, KAffine::translate((x0, y0)));
                }
            }
            None => self.content(n, xf, depth)?,
        }
        if isolated {
            self.pop_layer();
        }
        Ok(())
    }

    /// A group's inner shadow: cast inwards from around what its pixels cover, over them. The
    /// renderer's own `inner_shadow_of` works on the layer's pixels (read back once), so it lands
    /// where an export's does.
    fn group_inner_shadow(&mut self, layer: &Layer, s: &figcraft_doc::Shadow, scale: f64) -> Result<Layer, Unsupported> {
        let Some(pixels) = self.layers.layer_pixels(layer) else { return Err(Unsupported::Effect) };
        // The canvas draws this layer: its GPU may cast the shadow.
        let Some(shadow) = figcraft_render::shadow::inner_shadow_of(&pixels, scale, s, true) else { return Ok(layer.clone()) };
        let shadow = image_data(&Arc::new(shadow));
        let sampler = ImageSampler { x_extend: Extend::Pad, y_extend: Extend::Pad, quality: ImageQuality::Low, alpha: 1.0 };
        let mut scene = Scene::new();
        scene.draw_image(ImageBrush { image: &layer.image, sampler }, KAffine::IDENTITY);
        scene.draw_image(ImageBrush { image: &shadow, sampler }, KAffine::IDENTITY);
        self.layers.layer(&scene, layer.size.0, layer.size.1, None).ok_or(Unsupported::Effect)
    }

    /// The layer's texture effect: the renderer's own scatter of its pixels (read back once), so
    /// edges land where an export's do.
    fn texture_layer(
        &mut self,
        layer: &Layer,
        t: &figcraft_doc::Texture,
        layer_xf: Affine,
        seed: u32,
        key: Option<(u64, u64)>,
    ) -> Result<Layer, Unsupported> {
        let Some(mut pixels) = self.layers.layer_pixels(layer) else { return Err(Unsupported::Effect) };
        figcraft_render::grain::texture(&mut pixels, t, layer_xf, self.view.scale, seed);
        let image = image_data(&Arc::new(pixels));
        let sampler = ImageSampler { x_extend: Extend::Pad, y_extend: Extend::Pad, quality: ImageQuality::Low, alpha: 1.0 };
        let mut scene = Scene::new();
        scene.draw_image(ImageBrush { image: &image, sampler }, KAffine::IDENTITY);
        self.layers.layer(&scene, layer.size.0, layer.size.1, key).ok_or(Unsupported::Effect)
    }

    /// The layer's grain: the renderer's own noise from its pixels, over it, in the noise's blend
    /// mode. The grains are the CPU's own (a read-back of the layer), so they land where they do in
    /// an export.
    fn grain_layer(
        &mut self,
        layer: &Layer,
        nz: &figcraft_doc::Noise,
        layer_xf: Affine,
        seed: u32,
        key: Option<(u64, u64)>,
    ) -> Result<Layer, Unsupported> {
        let Some(pixels) = self.layers.layer_pixels(layer) else { return Err(Unsupported::Effect) };
        let Some(grains) = figcraft_render::grain::grains(&pixels, nz, layer_xf, seed) else { return Ok(layer.clone()) };
        let grains = image_data(&Arc::new(grains));
        let sampler = ImageSampler { x_extend: Extend::Pad, y_extend: Extend::Pad, quality: ImageQuality::Low, alpha: 1.0 };
        let mut scene = Scene::new();
        scene.draw_image(ImageBrush { image: &layer.image, sampler }, KAffine::IDENTITY);
        let blended = nz.blend != BlendMode::Normal && nz.blend != BlendMode::PassThrough;
        if blended {
            let target = KRect::new(0.0, 0.0, f64::from(layer.size.0), f64::from(layer.size.1));
            scene.push_layer(Fill::NonZero, blend(nz.blend), 1.0, KAffine::IDENTITY, &target);
        }
        scene.draw_image(ImageBrush { image: &grains, sampler }, KAffine::IDENTITY);
        if blended {
            scene.pop_layer();
        }
        self.layers.layer(&scene, layer.size.0, layer.size.1, key).ok_or(Unsupported::Effect)
    }

    /// Draw a shadow's pixels at `at` (device pixels), cut to `bounds` (the CPU crops it to the
    /// layer), outside `hole` (the node's paints) when given, in the shadow's own blend mode.
    fn draw_shadow_image(
        &mut self,
        image: &Layer,
        at: (f64, f64),
        bounds: Option<&KRect>,
        hole: Option<&figcraft_geom::BezPath>,
        mode: BlendMode,
        xf: Affine,
    ) {
        let image = &image.image;
        let Some(s) = &mut self.scene else { return };
        let blended = mode != BlendMode::Normal && mode != BlendMode::PassThrough;
        let target = KRect::new(0.0, 0.0, 32_768.0, 32_768.0);
        if blended {
            s.push_layer(Fill::NonZero, blend(mode), 1.0, KAffine::IDENTITY, &target);
        }
        let mut clips = 0;
        if let Some(r) = bounds {
            s.push_clip_layer(Fill::NonZero, KAffine::IDENTITY, &r.to_path(0.0));
            clips += 1;
        }
        if let Some(hole) = hole {
            s.push_clip_layer(Fill::EvenOdd, k_affine(xf), hole);
            clips += 1;
        }
        let sampler = ImageSampler { x_extend: Extend::Pad, y_extend: Extend::Pad, quality: ImageQuality::Low, alpha: 1.0 };
        s.draw_image(ImageBrush { image, sampler }, KAffine::translate(at));
        for _ in 0..clips {
            s.pop_layer();
        }
        if blended {
            s.pop_layer();
        }
    }

    /// A node's inner shadows: everything outside its shape at the shadow's offset (shrunk by the
    /// spread), blurred and tinted, over its fills and under its children and strokes.
    fn inner_shadows(&mut self, n: &Node, xf: Affine) -> Result<(), Unsupported> {
        let shadows: Vec<figcraft_doc::Shadow> = n
            .effects
            .iter()
            .filter(|e| e.visible)
            .filter_map(|e| match &e.kind {
                figcraft_doc::EffectKind::InnerShadow(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        if shadows.is_empty() {
            return Ok(());
        }
        // A group's inner shadow is cast from its pixels: that is the CPU's for now.
        let Some((shape, rule)) = node_outline(self.doc, n) else { return Err(Unsupported::Effect) };
        let Some((x0f, y0f, w, h)) = self.layer_rect_exact(n)? else { return Ok(()) };
        // The renderer's floored grid, as for a layer it draws as it is (`layer_pipeline`).
        let (x0, y0) = (x0f.floor(), y0f.floor());
        let scale = self.view.scale;
        let layer_xf = Affine::translate((-x0, -y0)) * xf;
        for (index, s) in shadows.iter().enumerate() {
            if !(s.offset.0.is_finite() && s.offset.1.is_finite() && s.radius.is_finite() && s.spread.is_finite()) {
                return Err(Unsupported::Effect);
            }
            let key = self.shadow_key(n, 1u64 << 32 | index as u64, layer_xf);
            let (image, at) = match self.layers.kept_shadow(key) {
                Some(image) => {
                    let (dx, dy) = (s.offset.0 * scale, s.offset.1 * scale);
                    let reach =
                        s.radius.abs() / 2.0 * scale * 3.0 + s.spread.abs() * scale * figcraft_doc::SPREAD_MITER_LIMIT + dx.abs().max(dy.abs());
                    let pad = if reach.is_finite() { reach.ceil().clamp(0.0, 2048.0) as u32 + 2 } else { 2 };
                    (image, (x0 - f64::from(pad), y0 - f64::from(pad)))
                }
                None => self.inner_shadow_image(&shape, rule, (x0f, y0f, w, h), layer_xf, scale, s, key)?,
            };
            let Some(scene) = &mut self.scene else { continue };
            let sampler = ImageSampler { x_extend: Extend::Pad, y_extend: Extend::Pad, quality: ImageQuality::Low, alpha: 1.0 };
            scene.push_clip_layer(fill_rule(rule), k_affine(xf), &shape);
            scene.draw_image(ImageBrush { image: &image.image, sampler }, KAffine::translate(at));
            scene.pop_layer();
        }
        Ok(())
    }

    /// One inner shadow's pixels and where they go: the shape's outside at its offset in a buffer
    /// padded by its reach, blurred and tinted (the builder clips them to the shape).
    fn inner_shadow_image(
        &mut self,
        shape: &figcraft_geom::BezPath,
        rule: FillRule,
        (x0, y0, w, h): (f64, f64, u32, u32),
        layer_xf: Affine,
        scale: f64,
        s: &figcraft_doc::Shadow,
        key: (u64, u64),
    ) -> Result<(Layer, (f64, f64)), Unsupported> {
        let (dx, dy) = (s.offset.0 * scale, s.offset.1 * scale);
        let spread = s.spread;
        // Everything outside the shape casts shadow inwards, from every edge: the buffer reaches as
        // far as the shadow does (the renderer's own reach).
        let reach = s.radius.abs() / 2.0 * scale * 3.0 + spread.abs() * scale * figcraft_doc::SPREAD_MITER_LIMIT + dx.abs().max(dy.abs());
        let pad = if reach.is_finite() { reach.ceil().clamp(0.0, 2048.0) as u32 + 2 } else { 2 };
        let (bw, bh) = (w.saturating_add(pad * 2).min(16_384), h.saturating_add(pad * 2).min(16_384));
        let big = KRect::new(0.0, 0.0, f64::from(bw), f64::from(bh));
        let at = KAffine::translate((f64::from(pad) + dx, f64::from(pad) + dy)) * k_affine(layer_xf);
        let white = PColor::new([1.0, 1.0, 1.0, 1.0]);
        let mut scene = Scene::new();
        scene.fill(Fill::NonZero, KAffine::IDENTITY, &Brush::Solid(white), None, &big);
        // Cut the shape (or, shrinking, the shape grown by the spread) out of the buffer.
        let band = Stroke::new((2.0 * spread.abs()).max(0.0)).with_join(vello::kurbo::Join::Miter).with_miter_limit(figcraft_doc::SPREAD_MITER_LIMIT);
        let place = |scene: &mut Scene| {
            scene.fill(fill_rule(rule), at, &Brush::Solid(white), None, shape);
            scene.stroke(&band, at, &Brush::Solid(white), None, shape);
        };
        scene.push_layer(Fill::NonZero, dest_out(), 1.0, KAffine::IDENTITY, &big);
        if spread > 0.0 {
            // The hole shrinks: only the shape goes, then the band comes back as its inner ring.
            scene.fill(fill_rule(rule), at, &Brush::Solid(white), None, shape);
        } else {
            place(&mut scene);
        }
        scene.pop_layer();
        if spread > 0.0 {
            scene.stroke(&band, at, &Brush::Solid(white), None, shape);
        }
        let sigma = s.radius / 2.0 * scale;
        let image = self.layers.shadow(&scene, bw, bh, sigma, s.color, Some(key)).ok_or(Unsupported::Effect)?;
        Ok((image, (x0 - f64::from(pad), y0 - f64::from(pad))))
    }

    /// A key for one of `n`'s shadows or layers: what the node draws (see [`content_hash`] — its
    /// own placement left out, so a drag keeps its pixels), `xf` (the layer's own placement in its
    /// layer: whole-pixel moves leave it, fractional ones change it), the shadow's place in the
    /// list — plus the document's shape (ids, node count), so another file's nodes at the same
    /// addresses can't match.
    fn shadow_key(&self, n: &Node, index: u64, xf: Affine) -> (u64, u64) {
        layer_key(self.doc, n, index, xf)
    }

    /// A layer for the node's opacity and blend mode, clipped to what the scene can see: the whole
    /// target (the builder doesn't know the view's bounds here; the canvas keeps its view).
    fn push_layer(&mut self, n: &Node) {
        self.push_blend_opacity(n.blend, n.opacity.clamp(0.0, 1.0));
    }

    /// A layer for a paint's own blend mode (`paint_layer`), clipped the same way.
    fn push_blend(&mut self, b: BlendMode) {
        self.push_blend_opacity(b, 1.0);
    }

    fn push_blend_opacity(&mut self, b: BlendMode, opacity: f32) {
        let Some(s) = &mut self.scene else { return };
        let target = KRect::new(0.0, 0.0, 32_768.0, 32_768.0);
        s.push_layer(Fill::NonZero, blend(b), opacity, KAffine::IDENTITY, &target);
    }

    fn pop_layer(&mut self) {
        if let Some(s) = &mut self.scene {
            s.pop_layer();
        }
    }

    /// Fills, children and strokes of `n`, without opacity, blend or effects.
    fn content(&mut self, n: &Node, xf: Affine, depth: usize) -> Result<(), Unsupported> {
        if matches!(n.kind, NodeKind::Page { .. } | NodeKind::Group) {
            return self.children(n.id, xf, depth + 1);
        }
        if matches!(n.kind, NodeKind::Text(_)) {
            return self.text(n, xf);
        }
        // The outline fills, clips children to rounded corners and strokes booleans.
        let outline = node_outline(self.doc, n);
        if let Some((path, rule)) = &outline
            && !path.is_empty()
        {
            for p in &n.fills {
                self.fill(p, path, *rule, (n.width, n.height), xf)?;
            }
        }
        // Inner shadows paint over the fills, under the children and strokes (the CPU's order).
        if n.effects.iter().any(|e| e.visible && matches!(e.kind, figcraft_doc::EffectKind::InnerShadow(_))) {
            self.inner_shadows(n, xf)?;
        }
        if !matches!(n.kind, NodeKind::BooleanOp { .. }) && !n.hides_contents() {
            let clips = n.clip && n.kind.is_frame_like() && !n.children.is_empty();
            let clip = if clips { outline.as_ref().map(|(p, rule)| (p.clone(), *rule)) } else { None };
            if let Some((p, rule)) = &clip
                && let Some(s) = &mut self.scene
            {
                s.push_clip_layer(fill_rule(*rule), k_affine(xf), p);
            }
            self.children(n.id, xf, depth + 1)?;
            if clip.is_some()
                && let Some(s) = &mut self.scene
            {
                s.pop_layer();
            }
        }
        if n.has_visible_stroke()
            && !self.no_strokes.contains(&n.id)
            && let Some((path, _)) = &outline
        {
            self.strokes(n, path, (n.width, n.height), xf, false)?;
        }
        Ok(())
    }

    /// A text node: one outline per run (a run with its own fills paints with them), the colour
    /// glyphs' own ink, the inner shadows over them, then the strokes — the renderer's order.
    fn text(&mut self, n: &Node, xf: Affine) -> Result<(), Unsupported> {
        let Some((layout, dy)) = figcraft_text::layout_node(n) else { return Ok(()) };
        let gxf = xf * Affine::translate((0.0, dy));
        let scale = gxf.determinant().abs().sqrt();
        let mut runs: std::collections::BTreeMap<usize, figcraft_geom::BezPath> = std::collections::BTreeMap::new();
        // Colour glyphs (emoji) carry their own ink, which draws instead of their outline.
        let mut ink: Vec<(usize, Ink)> = Vec::new();
        for g in &layout.glyphs {
            match g.color(scale) {
                Some(figcraft_text::ColorInk::Bitmap(bitmap, place)) => ink.push((g.style, Ink::Bitmap(bitmap, place))),
                Some(figcraft_text::ColorInk::Layers(layers)) => {
                    for layer in layers.iter() {
                        ink.push((g.style, Ink::Layer(g.layer_path(layer.gid), layer.rgba)));
                    }
                }
                None => runs.entry(g.style).or_default().extend(g.path().elements().iter().copied()),
            }
        }
        for d in &layout.decorations {
            runs.entry(d.style).or_default().extend(d.path().elements().iter().copied());
        }
        let size = (n.width, n.height);
        // Every run's fills (its ink included), in order: a run with ink and no outline draws only
        // that (a bitmap-only font).
        let mut styles: Vec<usize> = runs.keys().copied().chain(ink.iter().map(|(style, _)| *style)).collect();
        styles.sort_unstable();
        styles.dedup();
        for style in &styles {
            let fills = layout.styles.get(*style).and_then(|s| s.fills.as_ref()).unwrap_or(&n.fills);
            if let Some(path) = runs.get(style) {
                for p in fills {
                    self.fill(p, path, FillRule::Winding, size, gxf)?;
                }
            }
            for (_, item) in ink.iter().filter(|(s, _)| s == style) {
                match item {
                    Ink::Bitmap(bitmap, place) => self.glyph_bitmap(bitmap, gxf * *place),
                    Ink::Layer(path, rgba) => {
                        let c = Color::new(
                            f32::from(rgba[0]) / 255.0,
                            f32::from(rgba[1]) / 255.0,
                            f32::from(rgba[2]) / 255.0,
                            f32::from(rgba[3]) / 255.0,
                        );
                        self.fill(&Paint::solid(c), path, FillRule::Winding, size, gxf)?;
                    }
                }
            }
        }
        if n.effects.iter().any(|e| e.visible && matches!(e.kind, figcraft_doc::EffectKind::InnerShadow(_))) {
            self.inner_shadows(n, xf)?;
        }
        if n.has_visible_stroke() && !self.no_strokes.contains(&n.id) {
            // Glyph outlines are closed: inside and outside strokes clip to them.
            let rule = figcraft_render::stroke_rule(n, true);
            for path in runs.values() {
                let band = figcraft_render::stroke_area(n, path, true);
                for p in &n.strokes {
                    self.fill(p, band.as_ref(), rule, size, gxf)?;
                }
            }
        }
        Ok(())
    }

    /// A colour glyph's embedded bitmap (premultiplied RGBA) through `place`.
    fn glyph_bitmap(&mut self, bitmap: &Arc<figcraft_text::ColorBitmap>, place: Affine) {
        let (w, h) = (bitmap.width, bitmap.height);
        if w == 0 || h == 0 || bitmap.rgba.len() != (w as usize * h as usize * 4) {
            return;
        }
        // The layout caches the bitmaps, so the same one comes back every frame: its pixels are
        // handed to vello once.
        let key = (Arc::as_ptr(bitmap) as usize, w, h);
        let mut cache = glyph_images().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.len() >= MAX_IMAGES && !cache.contains_key(&key) {
            cache.clear();
        }
        let image = cache.entry(key).or_insert_with(|| ImageData {
            data: Blob::new(Arc::new(bitmap.rgba.clone())),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::AlphaPremultiplied,
            width: w,
            height: h,
        });
        let sampler = ImageSampler { x_extend: Extend::Pad, y_extend: Extend::Pad, quality: ImageQuality::Medium, alpha: 1.0 };
        let image: &ImageData = image;
        if let Some(s) = &mut self.scene {
            s.draw_image(ImageBrush { image, sampler }, k_affine(place));
        }
    }

    fn fill(&mut self, paint: &Paint, path: &figcraft_geom::BezPath, rule: FillRule, size: (f64, f64), xf: Affine) -> Result<(), Unsupported> {
        let Some(spec) = self.spec(paint, size, xf.determinant().abs().sqrt())? else { return Ok(()) };
        let isolated = paint_layer(paint.blend)?;
        if isolated {
            self.push_blend(paint.blend);
        }
        self.fill_spec(&spec, path, rule, xf);
        if isolated {
            self.pop_layer();
        }
        Ok(())
    }

    fn strokes(&mut self, n: &Node, path: &figcraft_geom::BezPath, size: (f64, f64), xf: Affine, text: bool) -> Result<(), Unsupported> {
        let props = &n.stroke;
        let scale = xf.determinant().abs().sqrt();
        let plain = props.align == figcraft_doc::StrokeAlign::Center
            && props.sides.is_none()
            && props.dashes.is_empty()
            && props.profile().is_none()
            && props.brush.is_none()
            && is_plain_cap(props.cap_start)
            && props.cap_start == props.cap_end;
        if text {
            // Glyph outlines are closed: inside and outside strokes clip to them.
            return Err(Unsupported::Stroke);
        }
        if plain {
            let stroke = Stroke::new(props.weight.max(0.0)).with_join(join(props.join)).with_caps(cap(props.cap_start));
            for p in &n.strokes {
                let Some(spec) = self.spec(p, size, scale)? else { continue };
                let isolated = paint_layer(p.blend)?;
                if isolated {
                    self.push_blend(p.blend);
                }
                self.stroke_spec(&spec, &stroke, path, xf);
                if isolated {
                    self.pop_layer();
                }
            }
            return Ok(());
        }
        // Everything else is the band the CPU paints — inside and outside strokes, per-side widths,
        // dashes, width profiles, brushes — as the renderer works its outline out, filled.
        let Some((band, rule)) = figcraft_render::stroke_fill(self.doc, n) else { return Err(Unsupported::Stroke) };
        for p in &n.strokes {
            let Some(spec) = self.spec(p, size, scale)? else { continue };
            let isolated = paint_layer(p.blend)?;
            if isolated {
                self.push_blend(p.blend);
            }
            self.fill_spec(&spec, band.as_ref(), rule, xf);
            if isolated {
                self.pop_layer();
            }
        }
        Ok(())
    }

    /// The paint's brush (`None`: it draws nothing), with its own blend mode's layer around it.
    fn spec(&self, paint: &Paint, size: (f64, f64), scale: f64) -> Result<Option<BrushSpec>, Unsupported> {
        if !paint.visible || paint.opacity <= 0.0 {
            return Ok(None);
        }
        let op = paint.opacity.clamp(0.0, 1.0);
        let spec = match &paint.kind {
            PaintKind::Solid { color } => {
                let c = color.with_alpha(color.a * op);
                if c.a <= 0.0 {
                    return Ok(None);
                }
                BrushSpec::Solid(pcolor(c))
            }
            PaintKind::Gradient(g) if g.kind != GradientKind::Diamond => {
                // vello's linear gradient is a projection onto its axis: a width handle that isn't
                // square to it (only imported files skew one) would draw different bands there.
                if !squared_linear(g, size) {
                    return Err(Unsupported::Paint);
                }
                BrushSpec::Gradient(gradient(g, op)?, Some(k_affine(handle_frame(g, size.0, size.1))))
            }
            // Images, pattern fills and diamond gradients are the renderer's own pixmaps — decoded,
            // filtered and cached there — drawn as vello's image brush through the renderer's own
            // image transform.
            PaintKind::Gradient(_) | PaintKind::Image(_) | PaintKind::Pattern(_) => {
                // The canvas is what this scene draws, so a large image comes as the smaller copy
                // the screen shows, as the CPU canvas draws it.
                let Some(Prepared::Pattern { pixmap, mode, quality: filter, opacity, transform }) =
                    prepare_at(paint, size.0, size.1, scale, self.doc, 0.0, true)
                else {
                    // A missing image, a pattern past its depth: the CPU paints nothing either.
                    return Ok(None);
                };
                let sampler = ImageSampler { x_extend: extend(mode), y_extend: extend(mode), quality: quality(filter), alpha: opacity };
                BrushSpec::Image(image_data(&pixmap), sampler, ts_affine(transform))
            }
        };
        Ok(Some(spec))
    }

    fn fill_spec(&mut self, spec: &BrushSpec, path: &figcraft_geom::BezPath, rule: FillRule, xf: Affine) {
        let Some(s) = &mut self.scene else { return };
        s.fill(fill_rule(rule), k_affine(xf), &spec.brush(), spec.transform(), path);
    }

    fn stroke_spec(&mut self, spec: &BrushSpec, stroke: &Stroke, path: &figcraft_geom::BezPath, xf: Affine) {
        let Some(s) = &mut self.scene else { return };
        s.stroke(stroke, k_affine(xf), &spec.brush(), spec.transform(), path);
    }
}

/// A colour glyph's own ink: an embedded bitmap and where it goes, or one COLR layer (its outline
/// and colour).
enum Ink {
    Bitmap(Arc<figcraft_text::ColorBitmap>, Affine),
    Layer(figcraft_geom::BezPath, [u8; 4]),
}

/// A paint's brush. Owned because an image's data lives in the cache, past the paint's life.
enum BrushSpec {
    Solid(PColor),
    Gradient(PGradient, Option<KAffine>),
    /// The image, how to sample it, and the image-pixel → local mapping.
    Image(ImageData, ImageSampler, KAffine),
}

impl BrushSpec {
    fn brush(&self) -> Brush {
        match self {
            BrushSpec::Solid(c) => Brush::Solid(*c),
            BrushSpec::Gradient(g, _) => Brush::Gradient(g.clone()),
            BrushSpec::Image(image, sampler, _) => Brush::Image(ImageBrush { image: image.clone(), sampler: *sampler }),
        }
    }

    /// The brush's own space → local mapping (`None`: the brush has no space of its own).
    fn transform(&self) -> Option<KAffine> {
        match self {
            BrushSpec::Solid(_) => None,
            BrushSpec::Gradient(_, t) => *t,
            BrushSpec::Image(_, _, t) => Some(*t),
        }
    }
}

/// Whether a paint's own blend mode needs a vello layer around it (`Err` for plus-darker, which
/// vello can't compose: the CPU draws it).
fn paint_layer(b: BlendMode) -> Result<bool, Unsupported> {
    if b == BlendMode::PlusDarker {
        return Err(Unsupported::Paint);
    }
    Ok(!matches!(b, BlendMode::PassThrough | BlendMode::Normal))
}

/// An image's identity in the caches below: the pixmap's (or bitmap's) address and its size.
type ImageKey = (usize, u32, u32);

/// The pixmaps handed to vello, by [`ImageKey`] (with the image the scene draws).
type ImageCache = Mutex<HashMap<ImageKey, (Arc<Pixmap>, ImageData)>>;

/// The pixmaps handed to vello, kept so their `Blob` outlives the frame: vello's atlas only keeps
/// an image's slot while the same blob is asked for, and a new one would upload it again.
fn images() -> &'static ImageCache {
    static IMAGES: OnceLock<ImageCache> = OnceLock::new();
    IMAGES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The most images kept (a page with more re-uploads them next frame: still correct, only slower).
const MAX_IMAGES: usize = 256;

/// Colour glyphs' bitmaps handed to vello, by the layout's own cached bitmap: kept so their `Blob`
/// outlives the frame (a new one would be uploaded again).
fn glyph_images() -> &'static Mutex<HashMap<ImageKey, ImageData>> {
    static GLYPHS: OnceLock<Mutex<HashMap<ImageKey, ImageData>>> = OnceLock::new();
    GLYPHS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `pm` as a vello image, cached by the pixmap's own identity (the renderer caches decoded images,
/// so a photo's pixmap is the same one every frame).
fn image_data(pm: &Arc<Pixmap>) -> ImageData {
    let key = (Arc::as_ptr(pm) as usize, pm.width(), pm.height());
    let mut cache = images().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, data)) = cache.get(&key) {
        return data.clone();
    }
    let data = ImageData {
        data: Blob::new(Arc::new(pm.data().to_vec())),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
        width: pm.width(),
        height: pm.height(),
    };
    if cache.len() >= MAX_IMAGES {
        cache.clear();
    }
    cache.insert(key, (pm.clone(), data.clone()));
    data
}

fn extend(mode: SpreadMode) -> Extend {
    match mode {
        SpreadMode::Pad => Extend::Pad,
        SpreadMode::Repeat => Extend::Repeat,
        SpreadMode::Reflect => Extend::Reflect,
    }
}

fn quality(q: FilterQuality) -> ImageQuality {
    match q {
        FilterQuality::Nearest => ImageQuality::Low,
        FilterQuality::Bilinear | FilterQuality::Bicubic => ImageQuality::Medium,
    }
}

/// tiny-skia's transform (image pixels → local) as kurbo's.
fn ts_affine(t: Transform) -> KAffine {
    KAffine::new([f64::from(t.sx), f64::from(t.ky), f64::from(t.kx), f64::from(t.sy), f64::from(t.tx), f64::from(t.ty)])
}

/// Whether a linear gradient's width handle is square to its axis in a `w` × `h` box — the way the
/// canvas keeps it, so its colour bands cross the axis at right angles (see
/// [`figcraft_color::Gradient::square_linear_width`]).
fn squared_linear(g: &Gradient, (w, h): (f64, f64)) -> bool {
    if g.kind != GradientKind::Linear {
        return true;
    }
    let (w, h) = (w.max(1e-6), h.max(1e-6));
    let d = ((g.to.0 - g.from.0) * w, (g.to.1 - g.from.1) * h);
    let k = ((g.width.0 - g.from.0) * w, (g.width.1 - g.from.1) * h);
    let len = (d.0 * d.0 + d.1 * d.1).sqrt() * (k.0 * k.0 + k.1 * k.1).sqrt();
    if !len.is_finite() || len <= 1e-12 {
        return true;
    }
    ((d.0 * k.0 + d.1 * k.1) / len).abs() <= 1e-6
}

/// A gradient brush in its unit box (`(0,0)` → from, `(1,0)` → to; see
/// [`figcraft_render::paint`]), which the brush transform maps onto the shape's box.
fn gradient(g: &Gradient, opacity: f32) -> Result<PGradient, Unsupported> {
    let kind = match g.kind {
        GradientKind::Linear => PGradient::new_linear(Point::new(0.0, 0.0), Point::new(1.0, 0.0)),
        GradientKind::Radial => PGradient::new_radial(Point::new(0.0, 0.0), 1.0),
        GradientKind::Angular => PGradient::new_sweep(Point::new(0.0, 0.0), 0.0, std::f32::consts::TAU),
        GradientKind::Diamond => return Err(Unsupported::Paint),
    };
    let mut stops = ColorStops::new();
    stops.extend(
        g.sorted_stops()
            .into_iter()
            .map(|s| ColorStop { offset: s.position, color: DynamicColor::from_alpha_color(pcolor(s.color.with_alpha(s.color.a * opacity))) }),
    );
    Ok(kind.with_extend(Extend::Pad).with_stops(stops))
}

/// The unit-box → local mapping for a gradient's handles (the renderer's `handle_frame`).
fn handle_frame(g: &Gradient, w: f64, h: f64) -> Affine {
    let w = w.max(1e-6);
    let h = h.max(1e-6);
    let f = (g.from.0 * w, g.from.1 * h);
    let t = (g.to.0 * w, g.to.1 * h);
    let k = (g.width.0 * w, g.width.1 * h);
    Affine::new([t.0 - f.0, t.1 - f.1, k.0 - f.0, k.1 - f.1, f.0, f.1])
}

/// The region outside `outlines` in local space: a huge rectangle with them as holes (even-odd).
fn outside(outlines: &[figcraft_render::shadow::Outline]) -> figcraft_geom::BezPath {
    let mut path = KRect::new(-1e5, -1e5, 1e5, 1e5).to_path(0.0);
    for (p, _) in outlines {
        path.extend(p.elements().iter().copied());
    }
    path
}

/// `outlines` grown (`spread > 0`) or shrunk by `spread` local units, shifted by `(dx, dy)` device
/// pixels, in white — the renderer's `shadow::spread_alpha`.
fn silhouette_scene(
    scene: &mut Scene,
    outlines: &[figcraft_render::shadow::Outline],
    spread: f64,
    layer_xf: Affine,
    (dx, dy): (f64, f64),
    rect: KRect,
) {
    let at = KAffine::translate((dx, dy)) * k_affine(layer_xf);
    let white = PColor::new([1.0, 1.0, 1.0, 1.0]);
    for (path, rule) in outlines {
        scene.fill(fill_rule(*rule), at, &Brush::Solid(white), None, path);
    }
    if !(spread.is_finite() && spread.abs() > 1e-6) {
        return;
    }
    let band = Stroke::new(2.0 * spread.abs()).with_join(vello::kurbo::Join::Miter).with_miter_limit(figcraft_doc::SPREAD_MITER_LIMIT);
    if spread > 0.0 {
        for (path, _) in outlines {
            scene.stroke(&band, at, &Brush::Solid(white), None, path);
        }
    } else {
        // Shrinking cuts the band out of the fill, as the renderer's clear pass does.
        scene.push_layer(Fill::NonZero, dest_out(), 1.0, KAffine::IDENTITY, &rect);
        for (path, _) in outlines {
            scene.stroke(&band, at, &Brush::Solid(white), None, path);
        }
        scene.pop_layer();
    }
}

/// The compose operator that takes what is drawn out of the layer under it.
fn dest_out() -> PBlend {
    PBlend::new(Mix::Normal, Compose::DestOut)
}

/// A device distance as a whole pixel offset (the renderer's `shadow::whole_px`).
fn whole_px(v: f64) -> i32 {
    if v.is_finite() { v.round().clamp(-100_000.0, 100_000.0) as i32 } else { 0 }
}

/// Where a shadow's pixels go from the node's layer corner, given how it is cast (a silhouette is
/// drawn with its offset baked in; pixels are moved by whole device pixels).
fn shadow_place(silhouette: &Option<Vec<figcraft_render::shadow::Outline>>, spread: f64, at: (f64, f64), (dx, dy): (f64, f64)) -> (f64, f64) {
    match silhouette {
        Some(_) if spread != 0.0 => at,
        _ => (at.0 + f64::from(whole_px(dx)), at.1 + f64::from(whole_px(dy))),
    }
}

/// Whether a shadow is cast from the node's outline grown by its spread rather than its pixels —
/// the CPU's `Caster` rule.
fn casts_from_outline(silhouette: &Option<Vec<figcraft_render::shadow::Outline>>, s: &figcraft_doc::Shadow) -> bool {
    silhouette.is_some() && s.spread != 0.0
}

/// What `n` draws, as one hash: its data with its own placement left out, and its children by
/// value (a child holds its own placement, which moves its pixels). A drag changes only the node's
/// translation — which the canvas applies where it draws the layer — so an effect layer keyed on
/// this stays valid across it, while any edit to what it draws changes the hash.
fn content_hash(doc: &Document, n: &Node, depth: usize) -> u64 {
    node_hash(doc, n, depth, false)
}

/// A node and everything under it, hashed by value (what it draws, not where it lives): two nodes
/// that draw the same thing hash the same, and an address the allocator hands out again cannot
/// pass for an unchanged node — what keying nodes by their `Arc` address got wrong once a drag
/// freed the last document. `own_transform` says whether the node's own placement is part of it.
fn node_hash(doc: &Document, n: &Node, depth: usize, own_transform: bool) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut copy = n.clone();
    if !own_transform {
        copy.transform = Affine::IDENTITY;
    }
    if serde_json::to_writer(HashSink(&mut h), &copy).is_err() {
        // Unserialisable is as good as changed: make the key unique.
        (n as *const Node as usize).hash(&mut h);
    }
    if depth < MAX_DEPTH {
        for c in &n.children {
            if let Some(cn) = doc.node(*c) {
                node_hash(doc, cn, depth + 1, true).hash(&mut h);
            }
        }
    }
    h.finish()
}

/// [`Builder::shadow_key`]: what `n` draws, its layer's placement, and the document's shape.
#[cfg(test)]
pub(crate) fn layer_key_for(doc: &Document, n: &Node, index: u64, xf: Affine) -> (u64, u64) {
    layer_key(doc, n, index, xf)
}

/// `id`'s layer rectangle at its exact device corner (tests: the layer's own placement).
#[cfg(test)]
pub(crate) fn layer_rect_for(doc: &Document, id: NodeId, view: &View) -> Option<(f64, f64, u32, u32)> {
    layer_rect_exact_impl(doc, doc.node(id)?, view).ok().flatten()
}

fn layer_key(doc: &Document, n: &Node, index: u64, xf: Affine) -> (u64, u64) {
    let mut first = std::collections::hash_map::DefaultHasher::new();
    content_hash(doc, n, 0).hash(&mut first);
    index.hash(&mut first);
    doc.pages.hash(&mut first);
    (doc.next_id(), doc.node_count()).hash(&mut first);
    (n.width.to_bits(), n.height.to_bits()).hash(&mut first);
    figcraft_text::layout_generation().hash(&mut first);
    doc.color_profile.hash(&mut first);
    let mut second = std::collections::hash_map::DefaultHasher::new();
    // The layer's transform, at a micro-pixel: the exact corner cancels the node's placement in
    // the reals, but composing the matrices rounds, so the last bits of the translation drift with
    // the node's position (at a zoom that isn't a power of two, and an origin that isn't whole).
    // Hashing them raw re-blurred the layer every frame of a drag; a micro-pixel is far below what
    // the layer's pixels can show.
    for v in xf.as_coeffs().map(|v| (v * 1e6).round() as i64) {
        v.hash(&mut second);
    }
    (first.finish(), second.finish())
}

/// A writer that feeds serialized bytes to a hasher (no allocation).
struct HashSink<'a>(&'a mut std::collections::hash_map::DefaultHasher);

impl std::io::Write for HashSink<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.write(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn fill_rule(r: FillRule) -> Fill {
    match r {
        FillRule::EvenOdd => Fill::EvenOdd,
        FillRule::Winding => Fill::NonZero,
    }
}

/// The view's rectangle in its own device pixels.
fn view_rect(view: &View) -> KRect {
    KRect::new(0.0, 0.0, f64::from(view.width), f64::from(view.height))
}

fn k_affine(a: Affine) -> KAffine {
    KAffine::new(a.as_coeffs())
}

/// `figcraft_color::Color` (sRGB, straight alpha) → peniko.
pub fn pcolor(c: Color) -> PColor {
    PColor::new([c.r, c.g, c.b, c.a])
}

fn blend(b: BlendMode) -> PBlend {
    use BlendMode as B;
    let mix = match b {
        B::PassThrough | B::Normal => Mix::Normal,
        B::Darken => Mix::Darken,
        B::Multiply => Mix::Multiply,
        B::ColorBurn => Mix::ColorBurn,
        B::Lighten => Mix::Lighten,
        B::Screen => Mix::Screen,
        B::ColorDodge => Mix::ColorDodge,
        B::Overlay => Mix::Overlay,
        B::SoftLight => Mix::SoftLight,
        B::HardLight => Mix::HardLight,
        B::Difference => Mix::Difference,
        B::Exclusion => Mix::Exclusion,
        B::Hue => Mix::Hue,
        B::Saturation => Mix::Saturation,
        B::Color => Mix::Color,
        B::Luminosity => Mix::Luminosity,
        B::PlusLighter => Mix::Normal,
        B::PlusDarker => Mix::Normal,
    };
    let compose = match b {
        B::PlusLighter => Compose::Plus,
        _ => Compose::SrcOver,
    };
    PBlend::new(mix, compose)
}

fn join(j: figcraft_doc::StrokeJoin) -> vello::kurbo::Join {
    match j {
        figcraft_doc::StrokeJoin::Miter => vello::kurbo::Join::Miter,
        figcraft_doc::StrokeJoin::Bevel => vello::kurbo::Join::Bevel,
        figcraft_doc::StrokeJoin::Round => vello::kurbo::Join::Round,
    }
}

fn is_plain_cap(c: figcraft_doc::StrokeCap) -> bool {
    matches!(c, figcraft_doc::StrokeCap::None | figcraft_doc::StrokeCap::Round | figcraft_doc::StrokeCap::Square)
}

fn cap(c: figcraft_doc::StrokeCap) -> vello::kurbo::Cap {
    match c {
        figcraft_doc::StrokeCap::Round => vello::kurbo::Cap::Round,
        figcraft_doc::StrokeCap::Square => vello::kurbo::Cap::Square,
        _ => vello::kurbo::Cap::Butt,
    }
}
