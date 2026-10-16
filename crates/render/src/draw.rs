//! Recursive scene drawing.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use figcraft_color::{BlendMode, Paint};
use figcraft_doc::{Document, EffectKind, Node, NodeId, NodeKind, StrokeAlign};
use figcraft_geom::{Affine, BezPath, Rect, Shape};
use tiny_skia::{FillRule, IntRect, Mask, MaskType, Pixmap, PixmapPaint, Transform};

use crate::backdrop::{self, Under};
use crate::boolean::{PathCache, locked};
use crate::paint::{composite, prepare_at, ts_blend};
use crate::{blur, grain, lod, shadow};

/// Shared state for one render.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub doc: &'a Document,
    /// Device pixels per document unit (for effect radii).
    pub scale: f64,
    pub outline_mode: bool,
    /// What outline mode draws besides shapes, and its line colour.
    pub outline: crate::Outline,
    /// Layers drawn in outline mode with their contents (outlines for the selection).
    pub outlined: &'a [NodeId],
    /// Nodes not to draw (e.g. text being edited draws through the editor).
    pub skip: &'a [NodeId],
    /// Approximate what's too small to see in detail (canvas previews; see [`crate::lod`]).
    pub lod: bool,
    /// The surfaces under the one being drawn into, when it is an isolated layer (for backdrops).
    pub under: Option<&'a Under<'a>>,
    /// Seconds into animated GIFs (0: their first frame; only the presenter plays them).
    pub time: f64,
}

pub fn ts_transform(a: Affine) -> Transform {
    let c = a.as_coeffs();
    Transform::from_row(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32, c[4] as f32, c[5] as f32)
}

/// kurbo path → tiny-skia path (None when empty or degenerate).
pub fn ts_path(p: &BezPath) -> Option<tiny_skia::Path> {
    use figcraft_geom::kurbo::PathEl;
    let mut b = tiny_skia::PathBuilder::new();
    for el in p.elements() {
        match *el {
            PathEl::MoveTo(p) => b.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => b.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(c, p) => b.quad_to(c.x as f32, c.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(c1, c2, p) => b.cubic_to(c1.x as f32, c1.y as f32, c2.x as f32, c2.y as f32, p.x as f32, p.y as f32),
            PathEl::ClosePath => b.close(),
        }
    }
    b.finish()
}

fn fill_with(pix: &mut Pixmap, path: &tiny_skia::Path, paints: &[Paint], w: f64, h: f64, xf: Affine, rule: FillRule, ctx: &Ctx, mask: Option<&Mask>) {
    let scale = xf.determinant().abs().sqrt();
    for p in paints {
        let Some(prep) = prepare_at(p, w, h, scale, ctx.doc, ctx.time) else { continue };
        let paint = tiny_skia::Paint { shader: prep.shader(), anti_alias: true, blend_mode: ts_blend(p.blend), ..Default::default() };
        if p.blend == BlendMode::PlusDarker {
            // No tiny-skia mode: the paint alone over its pixels, then blended in.
            let area = path
                .bounds()
                .transform(ts_transform(xf))
                .and_then(|b| int_rect(Rect::new(b.left().into(), b.top().into(), b.right().into(), b.bottom().into()), (pix.width(), pix.height())));
            let Some(mut alone) = area.and_then(|r| Pixmap::new(r.width(), r.height())) else { continue };
            let Some(r) = area else { continue };
            let t = ts_transform(Affine::translate((-f64::from(r.x()), -f64::from(r.y()))) * xf);
            alone.fill_path(path, &tiny_skia::Paint { blend_mode: tiny_skia::BlendMode::SourceOver, ..paint }, rule, t, None);
            crate::paint::composite(pix, (r.x(), r.y()), &alone, 1.0, p.blend);
            continue;
        }
        pix.fill_path(path, &paint, rule, ts_transform(xf), mask);
    }
}

/// Stroke outline cache: (geometry hash) → outline path.
fn stroke_cache() -> &'static Mutex<PathCache> {
    static C: OnceLock<Mutex<PathCache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(PathCache::new(4096)))
}

pub(crate) fn hash_path(p: &BezPath, h: &mut impl Hasher) {
    for el in p.elements() {
        use figcraft_geom::kurbo::PathEl::*;
        match *el {
            MoveTo(a) | LineTo(a) => (0u8, a.x.to_bits(), a.y.to_bits()).hash(h),
            QuadTo(a, b) => (1u8, a.x.to_bits(), a.y.to_bits(), b.x.to_bits(), b.y.to_bits()).hash(h),
            CurveTo(a, b, c) => (2u8, a.x.to_bits(), a.y.to_bits(), b.x.to_bits(), b.y.to_bits(), c.x.to_bits(), c.y.to_bits()).hash(h),
            ClosePath => 3u8.hash(h),
        }
    }
}

/// The stroke area of a node in local coordinates.
pub fn stroke_area(n: &Node, path: &BezPath, closed: bool) -> Arc<BezPath> {
    let props = &n.stroke;
    // Per-side strokes on rectangles/frames.
    if let Some(sides) = props.sides
        && matches!(n.kind, NodeKind::Rectangle | NodeKind::Frame | NodeKind::Component { .. } | NodeKind::Instance { .. })
        && n.corners.uniform() == Some(0.0)
    {
        return Arc::new(figcraft_pathops::rect_side_strokes(n.width, n.height, sides, props.align));
    }
    // Rect/ellipse inside/outside strokes without booleans: an even-odd ring of two outlines.
    if props.dashes.is_empty() && props.profile().is_none() && props.brush.is_none() && closed && props.align != StrokeAlign::Center {
        let w = props.weight;
        let ring = |inset: f64| -> Option<BezPath> {
            let (x0, y0, ww, hh) = (inset, inset, n.width - 2.0 * inset, n.height - 2.0 * inset);
            if ww <= 0.0 || hh <= 0.0 {
                return None;
            }
            match n.kind {
                NodeKind::Rectangle
                | NodeKind::Frame
                | NodeKind::Component { .. }
                | NodeKind::ComponentSet { .. }
                | NodeKind::Instance { .. }
                | NodeKind::Section { .. }
                    if n.corners.smoothing == 0.0 =>
                {
                    let radii = n.corners.radii.map(|r| if r > 0.0 { (r - inset).max(0.0) } else { 0.0 });
                    let mut p = figcraft_geom::rounded_rect(ww, hh, radii, 0.0);
                    p.apply_affine(Affine::translate((x0, y0)));
                    Some(p)
                }
                NodeKind::Ellipse { sweep, inner, .. } if sweep.abs() >= std::f64::consts::TAU - 1e-9 && inner == 0.0 => {
                    Some(figcraft_geom::kurbo::Ellipse::from_rect(Rect::new(x0, y0, x0 + ww, y0 + hh)).to_path(0.05))
                }
                _ => None,
            }
        };
        let (outer, inner) = match props.align {
            StrokeAlign::Inside => (ring(0.0), ring(w)),
            _ => (ring(-w), ring(0.0)),
        };
        if let Some(mut o) = outer {
            if let Some(i) = inner {
                o.extend(i.elements().iter().copied());
            }
            return Arc::new(o);
        }
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    hash_path(path, &mut h);
    (props.weight.to_bits(), props.align as u8, props.cap_start as u8, props.cap_end as u8, props.join as u8, props.miter_limit.to_bits(), closed)
        .hash(&mut h);
    for d in &props.dashes {
        d.to_bits().hash(&mut h);
    }
    props.dashes.len().hash(&mut h);
    // Points' own caps and joins (vector networks).
    let net = match &n.kind {
        NodeKind::Vector { network } if network.has_point_styles() => Some(network),
        _ => None,
    };
    for v in net.map(|n| n.vertices.as_slice()).unwrap_or_default() {
        (v.cap.map(|c| c as u8), v.join.map(|j| j as u8)).hash(&mut h);
    }
    for p in props.profile().map(|p| p.points.as_slice()).unwrap_or_default() {
        (p.t.to_bits(), p.scale.to_bits(), p.bias.to_bits()).hash(&mut h);
    }
    if let Some(b) = &props.brush {
        (&b.preset, b.spacing.to_bits(), b.size_jitter.to_bits(), b.rotation_jitter.to_bits(), b.scatter.to_bits(), b.seed).hash(&mut h);
    }
    let key = h.finish();
    if let Some(p) = locked(stroke_cache()).get(key) {
        return p;
    }
    let out = Arc::new(figcraft_pathops::stroke_outline_of(path, props, closed, net));
    locked(stroke_cache()).insert(key, out.clone());
    out
}

/// A node's painted stroke area in local coordinates with the rule to fill it by (`None` without a
/// visible stroke or geometry). Text strokes follow its glyphs and aren't covered.
pub fn stroke_fill(doc: &Document, n: &Node) -> Option<(Arc<BezPath>, FillRule)> {
    if !n.has_visible_stroke() {
        return None;
    }
    let closed = figcraft_doc::geometry::is_closed(n) || matches!(n.kind, NodeKind::BooleanOp { .. });
    let geom = match &n.kind {
        NodeKind::BooleanOp { .. } => node_outline(doc, n).map(|(p, _)| p),
        NodeKind::Text(_) | NodeKind::Group | NodeKind::Page { .. } => None,
        _ => figcraft_doc::geometry::stroke_geometry(n),
    }?;
    Some((stroke_area(n, &geom, closed), stroke_rule(n, closed)))
}

/// Even-odd ring paths need the even-odd rule; everything else non-zero.
pub(crate) fn stroke_rule(n: &Node, closed: bool) -> FillRule {
    if n.stroke.sides.is_some() || (n.stroke.dashes.is_empty() && closed && n.stroke.align != StrokeAlign::Center) {
        FillRule::EvenOdd
    } else {
        FillRule::Winding
    }
}

/// The geometry a node fills, in local space: shapes, booleans (computed from children) and text
/// (glyph outlines). Groups and pages have none.
pub fn node_outline(doc: &Document, n: &Node) -> Option<(BezPath, FillRule)> {
    node_outline_at(doc, n, 0)
}

/// [`node_outline`] `depth` levels into a boolean or group nesting (one budget for both).
fn node_outline_at(doc: &Document, n: &Node, depth: usize) -> Option<(BezPath, FillRule)> {
    match &n.kind {
        NodeKind::BooleanOp { op } => {
            let operands: Vec<(BezPath, figcraft_doc::BoolOp)> = n
                .children
                .iter()
                .filter_map(|c| doc.node(*c))
                .filter(|c| c.visible)
                .filter_map(|c| {
                    let (mut p, _) = subtree_outline(doc, c, depth + 1)?;
                    p.apply_affine(c.transform);
                    Some((p, c.bool_op.unwrap_or(*op)))
                })
                .collect();
            Some((crate::boolean::fold_cached(&operands), FillRule::EvenOdd))
        }
        NodeKind::Text(_) => {
            let (l, dy) = figcraft_text::layout_node(n)?;
            let mut p = l.to_path();
            p.apply_affine(Affine::translate((0.0, dy)));
            Some((p, FillRule::Winding))
        }
        NodeKind::Vector { network } => {
            let rule =
                if network.regions.iter().any(|r| r.winding == figcraft_doc::WindingRule::EvenOdd) { FillRule::EvenOdd } else { FillRule::Winding };
            Some((network.fill_path(), rule))
        }
        _ => figcraft_doc::geometry::shape_path(n).map(|p| (p, FillRule::Winding)),
    }
}

/// The union outline of a node and its descendants (for booleans of groups/frames).
pub fn subtree_outline(doc: &Document, n: &Node, depth: usize) -> Option<(BezPath, FillRule)> {
    if depth > 64 {
        return None;
    }
    match &n.kind {
        NodeKind::Group | NodeKind::Frame | NodeKind::Component { .. } | NodeKind::Instance { .. } if !n.children.is_empty() => {
            let mut out = BezPath::new();
            if n.kind.is_frame_like()
                && let Some(p) = figcraft_doc::geometry::shape_path(n)
                && n.fills.iter().any(|f| f.visible)
            {
                out.extend(p.elements().iter().copied());
            }
            for c in n.children.iter().filter_map(|c| doc.node(*c)) {
                if !c.visible {
                    continue;
                }
                if let Some((mut p, _)) = subtree_outline(doc, c, depth + 1) {
                    p.apply_affine(c.transform);
                    out.extend(p.elements().iter().copied());
                }
            }
            Some((out, FillRule::Winding))
        }
        _ => node_outline_at(doc, n, depth),
    }
}

fn device_rect(xf: Affine, r: Rect) -> Rect {
    figcraft_geom::transform_rect_bbox(xf, r)
}

/// The area text may paint without its glyphs being looked at: its box, generously grown.
fn text_box_bounds(n: &Node) -> Rect {
    n.local_rect().inflate(n.height.max(10.0), n.height.max(10.0) * 0.5)
}

/// Local bounds a node paints into (its box grown by stroke/effect reach, plus unclipped children).
/// Text that overflows its box counts where its glyphs are, so it isn't culled while visible;
/// text greeked at `greek_scale` (see [`lod::greeked`]) paints only inside [`text_box_bounds`].
fn local_paint_bounds(doc: &Document, n: &Node, depth: usize, greek_scale: Option<f64>) -> Rect {
    let reach = n.paint_reach();
    let mut r = n.local_rect().inflate(reach, reach);
    let clips = n.clip && n.kind.is_frame_like();
    if (!clips || n.kind.bounds_follow_children()) && depth < 64 && !n.hides_contents() {
        for c in n.children.iter().filter_map(|c| doc.node(*c)) {
            if !c.visible {
                continue;
            }
            let cb = local_paint_bounds(doc, c, depth + 1, greek_scale);
            r = r.union(figcraft_geom::transform_rect_bbox(c.transform, cb));
        }
    }
    // Text can overflow its box.
    if let NodeKind::Text(_) = n.kind {
        r = r.union(text_box_bounds(n).inflate(reach, reach));
        if greek_scale.is_some_and(|s| lod::greeked(n, s)) {
            return r;
        }
        if let Some((l, dy)) = figcraft_text::layout_node(n)
            && let Some(ink) = l.ink_bounds()
        {
            r = r.union(ink + figcraft_geom::kurbo::Vec2::new(0.0, dy));
        }
    }
    r
}

/// How far (document units) effects that read outside a node's paint bounds reach: blurs, glass,
/// drop shadows and texture. Culling and partial renders keep this much margin.
pub fn effect_margin(n: &Node) -> f64 {
    n.effects
        .iter()
        .filter(|e| e.visible)
        .map(|e| match &e.kind {
            EffectKind::LayerBlur(b) => b.max_radius() * 2.0,
            EffectKind::BackgroundBlur { .. } | EffectKind::Glass(_) => e.backdrop_radius() * 2.0,
            EffectKind::DropShadow(s) => s.radius * 2.0 + s.spread.abs() * figcraft_doc::SPREAD_MITER_LIMIT + s.offset.0.abs() + s.offset.1.abs(),
            EffectKind::Texture(t) => t.radius.abs(),
            // A group's inner shadows are cast from around its pixels: a layer cut off at the
            // view would cast one from the cut.
            EffectKind::InnerShadow(s) if matches!(n.kind, NodeKind::Group) => s.radius * 2.0 + s.spread.abs() + s.offset.0.abs() + s.offset.1.abs(),
            _ => 0.0,
        })
        .filter(|m| m.is_finite())
        .fold(0.0, f64::max)
}

/// Everything node `id` can paint, in page coordinates: its paint bounds (glyphs of overflowing
/// text included) grown by its effects' margin. What a change to the node can affect on screen.
pub fn paint_bounds(doc: &Document, id: NodeId) -> Option<Rect> {
    let n = doc.node(id)?;
    let m = effect_margin(n).abs();
    let local = local_paint_bounds(doc, n, 0, None);
    Some(figcraft_geom::transform_rect_bbox(doc.world_transform(id), local).inflate(m, m))
}

fn needs_layer(n: &Node) -> bool {
    n.opacity < 0.999
        || !matches!(n.blend, BlendMode::Normal | BlendMode::PassThrough)
        || n.effects.iter().any(|e| e.visible)
        || n.isolates_children()
}

/// `v` on the whole pixel that only float noise keeps it off: an edge on a pixel boundary comes out
/// a hair either side of it depending on the view's origin, and rounding outwards would then
/// take in a whole extra row or column in one render and not in another.
fn snap_px(v: f64) -> f64 {
    let r = v.round();
    if (v - r).abs() < 1e-6 { r } else { v }
}

fn int_rect(r: Rect, limit: (u32, u32)) -> Option<IntRect> {
    let x0 = snap_px(r.x0).floor().max(0.0);
    let y0 = snap_px(r.y0).floor().max(0.0);
    let x1 = snap_px(r.x1).ceil().min(f64::from(limit.0));
    let y1 = snap_px(r.y1).ceil().min(f64::from(limit.1));
    if !(x1 > x0 && y1 > y0) {
        return None;
    }
    IntRect::from_xywh(x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32)
}

/// The most a layer reaches past the view for its effects, in device pixels.
const MAX_EFFECT_MARGIN_PX: f64 = 2048.0;

/// Draw node `id` onto `pix`, where `parent_xf` maps the parent's space to `pix` pixels.
pub fn draw_node(ctx: &Ctx, pix: &mut Pixmap, id: NodeId, parent_xf: Affine, depth: usize) {
    if depth > crate::MAX_EXPORT_DEPTH || ctx.skip.contains(&id) {
        return;
    }
    let Some(n) = ctx.doc.node(id) else { return };
    let outline = ctx.outline_mode || ctx.outlined.contains(&id);
    if n.is_mask || n.opacity <= 0.0 && n.visible {
        return;
    }
    if !n.visible {
        // Outline mode can show hidden layers too (nothing of them is culled: a hidden layer
        // isn't counted in its parent's bounds).
        if outline && ctx.outline.hidden {
            draw_outline(&Ctx { outline_mode: true, ..*ctx }, pix, n, parent_xf * n.transform, depth);
        }
        return;
    }
    let xf = parent_xf * n.transform;
    let (pw, ph) = (pix.width(), pix.height());
    let local = local_paint_bounds(ctx.doc, n, 0, ctx.lod.then_some(ctx.scale));
    let dev = device_rect(xf, local);
    // Effects that read outside (blur) need a margin when culling. Absurd effects (a spread or
    // blur of millions of pixels) keep a sane layer: what lies further off reads as empty.
    let blur_margin = (effect_margin(n) * ctx.scale).min(MAX_EFFECT_MARGIN_PX);
    let view = Rect::new(0.0, 0.0, f64::from(pw), f64::from(ph)).inflate(blur_margin, blur_margin);
    if dev.intersect(view).is_zero_area() && !(dev.width() == 0.0 || dev.height() == 0.0) {
        return;
    }
    if outline {
        draw_outline(&Ctx { outline_mode: true, ..*ctx }, pix, n, xf, depth);
        return;
    }
    if ctx.lod && dev.width().max(dev.height()) < lod::TINY_PX && lod::splattable(ctx.doc, n, 0) {
        lod::splat_subtree(ctx, pix, n, xf, None, 1.0, depth);
        return;
    }
    if !needs_layer(n) {
        draw_content(ctx, pix, n, xf, depth, None);
        return;
    }
    // Isolated layer covering the node's paint bounds, clipped to the visible area + margin. Its
    // origin can be negative (inside the margin); the size counts from there, or a layer starting
    // above or left of the pixmap came out short by the margin (a band missing at its far edge).
    let vis = dev.intersect(view);
    let (x0, y0) = (vis.x0.floor(), vis.y0.floor());
    let (w, h) = (vis.x1.ceil() - x0, vis.y1.ceil() - y0);
    if !(x0.is_finite() && y0.is_finite() && w >= 1.0 && h >= 1.0) {
        return;
    }
    let (lx, ly) = (x0 as i32, y0 as i32);
    let lw = w.min(16_384.0) as u32;
    let lh = h.min(16_384.0) as u32;
    let Some(mut layer) = Pixmap::new(lw, lh) else { return };
    let lxf = Affine::translate((-f64::from(lx), -f64::from(ly))) * xf;
    let shape = node_outline(ctx.doc, n);
    // Background blur and glass: the whole backdrop under the layer (every surface below this
    // one, composited), through the shape, under the content.
    let back_shape = if backdrop::has_effects(n) { backdrop::outline(ctx.doc, n) } else { None };
    let read_backdrop = |pix: &Pixmap| back_shape.as_ref().and_then(|sh| backdrop_layer(ctx, pix, n, sh, (lx, ly), (lw, lh), lxf));
    // With drop shadows too, the layer casts them from what it paints, not from the backdrop, and
    // the backdrop is read again once they lie under it (a shadow showing behind is seen through
    // it). Until then, the layer's content sees the backdrop read first under it (#255).
    let casts = back_shape.is_some() && n.effects.iter().any(|e| e.visible && matches!(e.kind, EffectKind::DropShadow(_)));
    // Kept apart, the first read is only what backdrop effects inside the content see: without
    // any, the backdrop (blur or glass, the costly part) is computed once, after the shadows.
    let nested = || ctx.doc.subtree(id).iter().skip(1).any(|c| ctx.doc.node(*c).is_some_and(backdrop::has_effects));
    let first_back = if !casts || nested() { read_backdrop(pix) } else { None };
    if !casts && let Some(b) = &first_back {
        layer.draw_pixmap(0, 0, b.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }
    let op = n.opacity.clamp(0.0, 1.0);
    {
        // Inner shadows paint over the fills, under the children and strokes.
        let inner = |l: &mut Pixmap| {
            for e in n.effects.iter().filter(|e| e.visible) {
                if let EffectKind::InnerShadow(s) = &e.kind
                    && let Some(sh) = &shape
                    && let Some(inner) = shadow::inner_shadow(sh, lw, lh, lxf, ctx.scale, s)
                {
                    l.draw_pixmap(0, 0, inner.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                }
            }
        };
        let has_inner = shape.is_some() && n.effects.iter().any(|e| e.visible && matches!(e.kind, EffectKind::InnerShadow(_)));
        // Backdrops further down see this layer composited onto `pix` (over its backdrop, when
        // that is kept apart for now).
        let under = Under { opacity: op, blend: n.blend, ..Under::new(pix, (lx, ly), ctx.under) };
        let over_back = first_back.as_ref().filter(|_| casts).map(|b| Under::new(b, (0, 0), Some(&under)));
        let content_under = over_back.as_ref().unwrap_or(&under);
        draw_content(&Ctx { under: Some(content_under), ..*ctx }, &mut layer, n, lxf, depth, has_inner.then_some(&inner as &dyn Fn(&mut Pixmap)));
    }
    // A group is no outline of its own: its inner shadows are cast from around what its layers
    // draw together, over them.
    if matches!(n.kind, NodeKind::Group) && n.effects.iter().any(|e| e.visible && matches!(e.kind, EffectKind::InnerShadow(_))) {
        let cover = layer.clone();
        for e in n.effects.iter().filter(|e| e.visible) {
            if let EffectKind::InnerShadow(s) = &e.kind
                && let Some(sh) = shadow::inner_shadow_of(&cover, ctx.scale, s)
            {
                layer.draw_pixmap(0, 0, sh.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            }
        }
    }
    // Layer blur, noise and texture, in list order, over everything else.
    if !casts {
        layer_effects(ctx, &mut layer, n, id, lxf);
    }
    // Drop shadows go under the layer, in effect-list order.
    {
        // Kept apart, the backdrop isn't in the layer yet: the shadows are cast from the content
        // with the layer effects it will get (a copy only when it gets some).
        let layer_fx =
            n.effects.iter().any(|e| e.visible && matches!(e.kind, EffectKind::LayerBlur(_) | EffectKind::Noise(_) | EffectKind::Texture(_)));
        let content = (casts && layer_fx).then(|| {
            let mut c = layer.clone();
            layer_effects(ctx, &mut c, n, id, lxf);
            c
        });
        let caster = shadow::Caster::new(ctx.doc, n, content.as_ref().unwrap_or(&layer), lxf, ctx.scale);
        for e in n.effects.iter().filter(|e| e.visible) {
            if let EffectKind::DropShadow(s) = &e.kind
                && let Some(sh) = shadow::drop_shadow(&caster, s)
            {
                composite(pix, (lx, ly), &sh, op, s.blend);
            }
        }
    }
    if casts {
        // The backdrop with the shadows under it, the content over it, then the layer effects.
        if let Some(mut b) = read_backdrop(pix) {
            b.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            layer = b;
        }
        layer_effects(ctx, &mut layer, n, id, lxf);
    }
    composite(pix, (lx, ly), &layer, op, n.blend);
}

/// Node `n`'s backdrop effects through its backdrop outline `sh`, for its `size` layer at `at` on
/// `pix` (local → layer pixels through `lxf`): what `pix` holds there now, over what lies under it.
fn backdrop_layer(ctx: &Ctx, pix: &Pixmap, n: &Node, sh: &shadow::Outline, at: (i32, i32), size: (u32, u32), lxf: Affine) -> Option<Pixmap> {
    let ((lx, ly), (lw, lh)) = (at, size);
    let pad = backdrop::pad(n, ctx.scale);
    let p = i32::try_from(pad).unwrap_or(0);
    let back = backdrop::backdrop(pix, ctx.under, lx.saturating_sub(p), ly.saturating_sub(p), lw + pad * 2, lh + pad * 2)?;
    backdrop::effects(n, sh, &back, pad, lw, lh, lxf, ctx.scale)
}

/// Node `n`'s layer blur, noise and texture on its `layer`, in list order.
fn layer_effects(ctx: &Ctx, layer: &mut Pixmap, n: &Node, id: NodeId, lxf: Affine) {
    for e in n.effects.iter().filter(|e| e.visible) {
        match &e.kind {
            EffectKind::LayerBlur(b) => blur::apply(layer, b, (n.width, n.height), lxf, ctx.scale),
            EffectKind::Noise(nz) => grain::noise(layer, nz, lxf, id.0 as u32),
            EffectKind::Texture(t) => grain::texture(layer, t, lxf, ctx.scale, id.0 as u32),
            _ => {}
        }
    }
}

fn draw_outline(ctx: &Ctx, pix: &mut Pixmap, n: &Node, xf: Affine, depth: usize) {
    let color = if ctx.outline.light { tiny_skia::Color::WHITE } else { tiny_skia::Color::BLACK };
    let paint = tiny_skia::Paint { shader: tiny_skia::Shader::SolidColor(color), anti_alias: true, ..Default::default() };
    let stroke = tiny_skia::Stroke { width: 1.0 / ctx.scale.max(1e-6) as f32, ..Default::default() };
    if let Some((path, _)) = node_outline(ctx.doc, n)
        && !matches!(n.kind, NodeKind::Group)
        && let Some(tp) = ts_path(&path)
    {
        pix.stroke_path(&tp, &paint, &stroke, ts_transform(xf), None);
    }
    // Object bounds: the box of every layer whose outline isn't its box already.
    let boxed = matches!(n.kind, NodeKind::Rectangle) && n.corners.uniform() == Some(0.0) || n.kind.is_frame_like();
    if ctx.outline.bounds
        && !boxed
        && let Some(tp) = ts_path(&figcraft_geom::kurbo::Shape::to_path(&n.local_rect(), 0.1))
    {
        pix.stroke_path(&tp, &paint, &stroke, ts_transform(xf), None);
    }
    if !matches!(n.kind, NodeKind::BooleanOp { .. }) && !n.hides_contents() {
        for c in n.paint_order() {
            draw_node(ctx, pix, c, xf, depth + 1);
        }
    }
}

/// What paints over a node's fills, under its children and strokes (its inner shadows).
type OverFills<'a> = Option<&'a dyn Fn(&mut Pixmap)>;

/// Fills, `over_fills`, children (clipped), strokes and text of a node, without opacity/effects.
fn draw_content(ctx: &Ctx, pix: &mut Pixmap, n: &Node, xf: Affine, depth: usize, over_fills: OverFills) {
    let doc = ctx.doc;
    let (w, h) = (n.width, n.height);
    match &n.kind {
        NodeKind::Page { .. } => {
            draw_children(ctx, pix, n, xf, depth);
        }
        NodeKind::Group => draw_children(ctx, pix, n, xf, depth),
        NodeKind::Text(_) if ctx.lod && lod::greeked(n, ctx.scale) => {
            lod::greek_text(pix, n, xf, text_box_bounds(n));
            if let Some(f) = over_fills {
                f(pix);
            }
        }
        NodeKind::Text(t) => {
            let Some((layout, dy)) = figcraft_text::layout_node(n) else { return };
            let gxf = xf * Affine::translate((0.0, dy));
            // Colour glyphs (emoji) carry their own ink, which draws instead of their outline: a
            // bitmap the font ships, or COLR layers with a colour each.
            let gs = gxf.determinant().abs().sqrt();
            let mut ink: Vec<(usize, Ink)> = Vec::new();
            // Group glyphs by style so runs with their own fills paint with them.
            let mut by_style: HashMap<usize, BezPath> = HashMap::new();
            for g in &layout.glyphs {
                match g.color(gs) {
                    Some(figcraft_text::ColorInk::Bitmap(bitmap, place)) => ink.push((g.style, Ink::Bitmap(bitmap, place))),
                    Some(figcraft_text::ColorInk::Layers(layers)) => {
                        for layer in layers.iter() {
                            ink.push((g.style, Ink::Layer(g.layer_path(layer.gid), layer.rgba)));
                        }
                    }
                    None => by_style.entry(g.style).or_default().extend(g.path().elements().iter().copied()),
                }
            }
            for d in &layout.decorations {
                by_style.entry(d.style).or_default().extend(d.path().elements().iter().copied());
            }
            // A run can have ink and no outline at all (bitmap-only fonts).
            let mut keys: Vec<usize> = by_style.keys().copied().chain(ink.iter().map(|(style, _)| *style)).collect();
            keys.sort_unstable();
            keys.dedup();
            // Every run's fills (its ink included), then what paints over the fills, then the
            // strokes on top.
            for k in &keys {
                let fills = layout.styles.get(*k).and_then(|s| s.fills.as_ref()).unwrap_or(&n.fills);
                if let Some(tp) = by_style.get(k).and_then(ts_path) {
                    fill_with(pix, &tp, fills, w, h, gxf, FillRule::Winding, ctx, None);
                }
                for (_, item) in ink.iter().filter(|(style, _)| style == k) {
                    match item {
                        Ink::Bitmap(bitmap, place) => blit(pix, bitmap, ts_transform(gxf * *place)),
                        Ink::Layer(path, rgba) => {
                            if let Some(tp) = ts_path(path) {
                                let mut paint = tiny_skia::Paint::default();
                                paint.set_color_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]);
                                paint.anti_alias = true;
                                pix.fill_path(&tp, &paint, FillRule::Winding, ts_transform(gxf), None);
                            }
                        }
                    }
                }
            }
            if let Some(f) = over_fills {
                f(pix);
            }
            if n.has_visible_stroke() {
                let props = figcraft_doc::StrokeProps { align: StrokeAlign::Center, ..n.stroke.clone() };
                for path in keys.iter().filter_map(|k| by_style.get(k)) {
                    let s = figcraft_pathops::stroke_center(path, props.weight, &props);
                    if let Some(ts) = ts_path(&s) {
                        fill_with(pix, &ts, &n.strokes, w, h, gxf, FillRule::Winding, ctx, None);
                    }
                }
            }
            let _ = t;
        }
        _ => {
            let closed = figcraft_doc::geometry::is_closed(n) || matches!(n.kind, NodeKind::BooleanOp { .. });
            let splatted = ctx.lod && closed && lod::splat_fills(pix, n, xf);
            let clips = n.clip && n.kind.is_frame_like() && !n.children.is_empty();
            // The outline fills, clips children to rounded corners and strokes booleans.
            let outline = if splatted && !(clips && n.corners.uniform() != Some(0.0)) { None } else { node_outline(doc, n) };
            if !splatted
                && let Some((path, rule)) = &outline
                && closed
                && let Some(tp) = ts_path(path)
            {
                fill_with(pix, &tp, &n.fills, w, h, xf, *rule, ctx, None);
            }
            if let Some(f) = over_fills {
                f(pix);
            }
            if !matches!(n.kind, NodeKind::BooleanOp { .. }) && !n.children.is_empty() && !n.hides_contents() {
                if clips {
                    draw_children_clipped(ctx, pix, n, xf, depth, outline.as_ref().map(|(p, _)| p));
                } else {
                    draw_children(ctx, pix, n, xf, depth);
                }
            }
            if n.has_visible_stroke() {
                let geom = match &n.kind {
                    NodeKind::BooleanOp { .. } => outline.as_ref().map(|(p, _)| p.clone()),
                    _ => figcraft_doc::geometry::stroke_geometry(n),
                };
                if let Some(g) = geom {
                    let area = stroke_area(n, &g, closed);
                    if let Some(tp) = ts_path(&area) {
                        fill_with(pix, &tp, &n.strokes, w, h, xf, stroke_rule(n, closed), ctx, None);
                    }
                }
            }
        }
    }
}

fn draw_children(ctx: &Ctx, pix: &mut Pixmap, n: &Node, xf: Affine, depth: usize) {
    let kids = &n.children;
    for run in ctx.doc.paint_runs(n) {
        let Some(&c) = kids.get(run.start) else { continue };
        if ctx.doc.node(c).is_some_and(|cn| cn.is_mask && cn.visible) {
            // Everything above the mask (until the next mask) is clipped by its alpha.
            draw_masked(ctx, pix, c, kids.get(run.start + 1..run.end).unwrap_or(&[]), xf, depth);
        } else {
            draw_node(ctx, pix, c, xf, depth + 1);
        }
    }
}

fn draw_masked(ctx: &Ctx, pix: &mut Pixmap, mask_id: NodeId, content: &[NodeId], xf: Affine, depth: usize) {
    let (w, h) = (pix.width(), pix.height());
    let Some(mut mask_px) = Pixmap::new(w, h) else { return };
    let mask_type = ctx.doc.node(mask_id).map(|m| m.mask_type).unwrap_or_default();
    // Draw the mask node as if it weren't a mask (a vector mask: its outline, fully opaque).
    if let Some(m) = ctx.doc.node(mask_id) {
        let mut m2 = m.clone();
        m2.is_mask = false;
        let mxf = xf * m2.transform;
        if mask_type == figcraft_doc::MaskType::Vector {
            if let Some((outline, rule)) = subtree_outline(ctx.doc, &m2, depth + 1)
                && let Some(path) = ts_path(&outline)
            {
                let mut paint = tiny_skia::Paint::default();
                paint.set_color_rgba8(255, 255, 255, 255);
                paint.anti_alias = true;
                mask_px.fill_path(&path, &paint, rule, ts_transform(mxf), None);
            }
        } else if needs_layer(&m2) {
            let paint = PixmapPaint { opacity: m2.opacity, ..Default::default() };
            let mut tmp = Pixmap::new(w, h);
            if let Some(tmp) = tmp.as_mut() {
                draw_content(ctx, tmp, &m2, mxf, depth + 1, None);
                mask_px.draw_pixmap(0, 0, tmp.as_ref(), &paint, Transform::identity(), None);
            }
        } else {
            draw_content(ctx, &mut mask_px, &m2, mxf, depth + 1, None);
        }
    }
    let Some(mut layer) = Pixmap::new(w, h) else { return };
    let kind = if mask_type == figcraft_doc::MaskType::Luminance { MaskType::Luminance } else { MaskType::Alpha };
    let mask = Mask::from_pixmap(mask_px.as_ref(), kind);
    {
        let under = Under { clip: Some(&mask), ..Under::new(pix, (0, 0), ctx.under) };
        let sub = Ctx { under: Some(&under), ..*ctx };
        for c in content {
            draw_node(&sub, &mut layer, *c, xf, depth + 1);
        }
    }
    pix.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), Some(&mask));
}

fn draw_children_clipped(ctx: &Ctx, pix: &mut Pixmap, n: &Node, xf: Affine, depth: usize, clip: Option<&BezPath>) {
    let (pw, ph) = (pix.width(), pix.height());
    let dev = device_rect(xf, n.local_rect());
    let Some(ir) = int_rect(dev, (pw, ph)) else { return };
    let (lx, ly) = (ir.x(), ir.y());
    // The layer starts as what lies under it and goes back in its place through the clip:
    // clipping cuts the children off without isolating them, so their blend modes reach the
    // layers behind the frame.
    let Some(mut layer) = pix.clone_rect(ir) else { return };
    let lxf = Affine::translate((-f64::from(lx), -f64::from(ly))) * xf;
    let square_aligned = n.corners.uniform() == Some(0.0) && is_axis_aligned(xf);
    let rect_clip = square_aligned && on_whole_pixels(dev);
    // The clip (a rectangular one on whole pixels is the layer's own box) is known before the
    // children draw, so their backdrops are clipped the way the layer will be. An axis-aligned
    // square frame with fractional edges gets an anti-aliased mask of its box so its edge pixels
    // are only partly covered.
    let mask = if rect_clip {
        None
    } else {
        let own_box = (square_aligned && clip.is_none()).then(|| n.local_rect().to_path(0.1));
        clip.or(own_box.as_ref()).and_then(ts_path).and_then(|cp| {
            let mut m = Mask::new(ir.width(), ir.height())?;
            m.fill_path(&cp, FillRule::Winding, true, ts_transform(lxf));
            Some(m)
        })
    };
    if !rect_clip && mask.is_none() {
        return;
    }
    {
        let under = Under { clip: mask.as_ref(), replaces: true, ..Under::new(pix, (lx, ly), ctx.under) };
        draw_children(&Ctx { under: Some(&under), ..*ctx }, &mut layer, n, lxf, depth);
    }
    backdrop::replace(pix, (lx, ly), &layer, mask.as_ref().map(|m| (m, (0, 0))));
}

/// Colour ink of a text run, in the order the run paints it. Strokes follow the outlines only: a
/// bitmap-strike glyph has no outline to stroke.
enum Ink {
    /// An embedded bitmap and the transform from its pixels into layout coordinates.
    Bitmap(Arc<figcraft_text::ColorBitmap>, Affine),
    /// One COLR layer: its outline in layout coordinates and the colour to fill it with.
    Layer(BezPath, [u8; 4]),
}

/// Draw a colour bitmap (premultiplied RGBA) under a transform, sampled smoothly when the
/// transform scales it.
fn blit(pix: &mut Pixmap, bitmap: &figcraft_text::ColorBitmap, transform: Transform) {
    let Some(image) = tiny_skia::PixmapRef::from_bytes(&bitmap.rgba, bitmap.width, bitmap.height) else { return };
    let paint = PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
    pix.draw_pixmap(0, 0, image, &paint, transform, None);
}

fn is_axis_aligned(xf: Affine) -> bool {
    let c = xf.as_coeffs();
    c[1].abs() < 1e-9 && c[2].abs() < 1e-9 && fract_ok(c[4]) && fract_ok(c[5])
}

fn fract_ok(_v: f64) -> bool {
    true
}

/// Whether all four edges of the device rectangle `r` lie on whole pixels, up to the float noise
/// `snap_px` forgives.
fn on_whole_pixels(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].into_iter().all(|v| (snap_px(v) - v.round()).abs() < 1e-9)
}
