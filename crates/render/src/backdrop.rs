//! Backdrops: what lies under a layer, for the effects that show it (background blur and glass).
//!
//! A render draws some layers into surfaces of their own (opacity, blend modes and effects,
//! clipping frames, masks) and composites each onto the surface below once it is done. A layer's
//! backdrop is everything drawn under it so far on all of those surfaces, composited the way they
//! will be: the frame's own fill under a clipped child, the page under that. [`Under`] records the
//! stack of surfaces as the render descends; [`backdrop`] flattens it for a rectangle.
//!
//! The PDF and SVG exports, which have no pixels to read, render what lies under the layer instead
//! ([`effects_beneath`]) and draw the same [`effects`] from it.

use figcraft_doc::{Document, EffectKind, Node, NodeId, NodeKind};
use figcraft_geom::{Affine, Rect};
use tiny_skia::{Mask, Pixmap, PixmapPaint, Transform};

use crate::shadow::{Outline, spread_alpha};

/// Deepest ancestry walked (the renderer's nesting limit).
const MAX_DEPTH: usize = 256;
use crate::{View, fx, glass};

/// One isolated surface of a render, and how it will be composited onto the surface below.
#[derive(Clone, Copy)]
pub struct Under<'a> {
    /// The surface below.
    pub pix: &'a Pixmap,
    /// Where this surface's top-left pixel lands on `pix`.
    pub at: (i32, i32),
    pub opacity: f32,
    pub blend: figcraft_color::BlendMode,
    /// What this surface is clipped by when composited, in its own pixels.
    pub clip: Option<&'a Mask>,
    /// The surface started as a copy of `pix` there and goes back in its place through `clip`
    /// (a clipping frame's children: clipping doesn't isolate them), instead of over it.
    pub replaces: bool,
    /// The surface under `pix`, if `pix` is itself an isolated surface.
    pub below: Option<&'a Under<'a>>,
}

impl<'a> Under<'a> {
    /// A surface drawn at `at` onto `pix` (itself over `below`), as is.
    pub fn new(pix: &'a Pixmap, at: (i32, i32), below: Option<&'a Under<'a>>) -> Self {
        Under { pix, at, opacity: 1.0, blend: figcraft_color::BlendMode::Normal, clip: None, replaces: false, below }
    }
}

/// Whether these surfaces are copies of their parent rather than isolated content. A backdrop
/// replaces the pixels it samples on such surfaces. An isolated layer still captures its
/// backdrop as content, to which that layer's own opacity, blend and mask apply later.
/// This deliberately preserves the existing capture semantics inside isolated groups and masks:
/// replacing their backdrop would require carrying replacement coverage through their ancestors.
pub(crate) fn replaces_backdrop(mut under: Option<&Under>) -> bool {
    for _ in 0..=MAX_DEPTH {
        let Some(u) = under else { return true };
        if !u.replaces {
            return false;
        }
        under = u.below;
    }
    false
}

/// Coverage of a layer whose backdrop replaces the original pixels. The backdrop's alpha is
/// sampled transparency, while the shape's alpha says how much of the original is replaced.
/// Recover the content's remaining transparency from its composite over `back`; this also
/// preserves fills, strokes and children extending outside the backdrop outline.
/// In normalized alpha, source = backdrop + content * (1 - backdrop), whereas replacement
/// coverage = shape + content * (1 - shape). Their difference is therefore
/// (shape - backdrop) * (1 - source) / (1 - backdrop), including antialiased shape edges.
/// Returns `None` when source alpha already describes all the coverage.
pub(crate) fn replacement_coverage(shape: &Outline, back: &Pixmap, layer: &Pixmap, xf: Affine) -> Option<Pixmap> {
    let mut cover = spread_alpha(std::slice::from_ref(shape), 0.0, xf, layer.width(), layer.height())?;
    let mut differs = false;
    for ((c, b), s) in cover.data_mut().chunks_exact_mut(4).zip(back.data().chunks_exact(4)).zip(layer.data().chunks_exact(4)) {
        let (shape, backdrop, source) = (u32::from(c[3]), u32::from(b[3]), u32::from(s[3]));
        let extra = if backdrop < 255 {
            let remaining = 255 - backdrop;
            (shape.saturating_sub(backdrop) * (255 - source) + remaining / 2) / remaining
        } else {
            0
        };
        differs |= extra > 0;
        c.copy_from_slice(&[0, 0, 0, (source + extra).min(255) as u8]);
    }
    differs.then_some(cover)
}

/// Composite a normal layer with independent replacement coverage. Transparent pixels sampled
/// by blur or glass must remove the original sharp pixels as well as contribute their colour.
pub(crate) fn composite_replacing(dst: &mut Pixmap, at: (i32, i32), src: &Pixmap, cover: &Pixmap, opacity: f32) {
    let opacity = opacity.clamp(0.0, 1.0);
    let width = cover.width() as usize;
    fx::overlap_rows(dst, at, src, |dp, sp, row, col| {
        let start = (row as usize * width + col as usize) * 4;
        let Some(cp) = cover.data().get(start..start + sp.len()) else { return };
        for ((d, s), c) in dp.chunks_exact_mut(4).zip(sp.chunks_exact(4)).zip(cp.chunks_exact(4)) {
            let keep = 1.0 - opacity * f32::from(c[3].max(s[3])) / 255.0;
            for (d, s) in d.iter_mut().zip(s) {
                *d = (f32::from(*s) * opacity + f32::from(*d) * keep).round().clamp(0.0, 255.0) as u8;
            }
        }
    });
}

/// Multiply `p` (the pixels at `(x, y)` of a surface) by the coverage of `m` (that surface's clip).
fn clip_at(p: &mut Pixmap, m: &Mask, x: i32, y: i32) {
    let (mw, mh) = (i64::from(m.width()), i64::from(m.height()));
    let w = p.width() as usize;
    let md = m.data();
    for (i, px) in p.data_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (mx, my) = (i64::from(x) + (i % w) as i64, i64::from(y) + (i / w) as i64);
        let c = if mx >= 0 && my >= 0 && mx < mw && my < mh { u32::from(md.get((my * mw + mx) as usize).copied().unwrap_or(0)) } else { 0 };
        if c < 255 {
            for v in px.iter_mut() {
                *v = ((u32::from(*v) * c + 127) / 255) as u8;
            }
        }
    }
}

/// The backdrop of the `w` × `h` pixel rectangle at `(x, y)` of `pix`: what is drawn on `pix` so
/// far, over everything under it.
pub fn backdrop(pix: &Pixmap, under: Option<&Under>, x: i32, y: i32, w: u32, h: u32) -> Option<Pixmap> {
    onto(copy_rect(pix, x, y, w, h)?, under, x, y)
}

/// The `w` × `h` pixels of `pix` at `(x, y)` (transparent past its edges).
fn copy_rect(pix: &Pixmap, x: i32, y: i32, w: u32, h: u32) -> Option<Pixmap> {
    let mut part = Pixmap::new(w, h)?;
    let copy = PixmapPaint { blend_mode: tiny_skia::BlendMode::Source, ..Default::default() };
    part.draw_pixmap(x.checked_neg()?, y.checked_neg()?, pix.as_ref(), &copy, Transform::identity(), None);
    Some(part)
}

/// `part`, the pixels at `(x, y)` of a surface, composited onto the surfaces `under` it.
fn onto(mut part: Pixmap, under: Option<&Under>, x: i32, y: i32) -> Option<Pixmap> {
    let Some(u) = under else { return Some(part) };
    let (w, h) = (part.width(), part.height());
    let (ux, uy) = (x.checked_add(u.at.0)?, y.checked_add(u.at.1)?);
    if u.replaces {
        // `part` takes the place of `u.pix`'s pixels through the clip, then goes where they go.
        let mut merged = copy_rect(u.pix, ux, uy, w, h)?;
        replace(&mut merged, (0, 0), &part, u.clip.map(|m| (m, (x, y))));
        return onto(merged, u.below, ux, uy);
    }
    let mut out = backdrop(u.pix, u.below, ux, uy, w, h)?;
    if let Some(m) = u.clip {
        clip_at(&mut part, m, x, y);
    }
    crate::paint::composite(&mut out, (0, 0), &part, u.opacity, u.blend);
    Some(out)
}

/// Put `src` in place of the pixels of `dst` at `at`, through `clip` (a mask and where `src`'s
/// top-left lies on it): `dst` keeps what the clip leaves out, and pixels past either edge are
/// dropped.
pub(crate) fn replace(dst: &mut Pixmap, at: (i32, i32), src: &Pixmap, clip: Option<(&Mask, (i32, i32))>) {
    fx::overlap_rows(dst, at, src, |dp, sp, r, c0| {
        let Some((m, (mx, my))) = clip else { return dp.copy_from_slice(sp) };
        let (mw, mh, my) = (i64::from(m.width()), i64::from(m.height()), r + i64::from(my));
        for (c, (d, s)) in (c0..).zip(dp.chunks_exact_mut(4).zip(sp.chunks_exact(4))) {
            let mx = c + i64::from(mx);
            let k = if mx >= 0 && my >= 0 && mx < mw && my < mh { u32::from(m.data().get((my * mw + mx) as usize).copied().unwrap_or(0)) } else { 0 };
            for (d, s) in d.iter_mut().zip(s) {
                *d = ((u32::from(*d) * (255 - k) + u32::from(*s) * k + 127) / 255) as u8;
            }
        }
    });
}

/// Whether a node has a visible background blur or glass.
pub fn has_effects(n: &Node) -> bool {
    n.effects.iter().any(|e| (e.visible && matches!(e.kind, EffectKind::Glass(_))) || e.backdrop_radius() > 0.0)
}

/// Pixels around a layer its backdrop effects read at `scale`.
pub fn pad(n: &Node, scale: f64) -> u32 {
    let r = n.effects.iter().map(|e| crate::backdrop_reach(e.backdrop_radius(), scale)).fold(0.0, f64::max);
    if r.is_finite() { r.ceil().clamp(0.0, 2048.0) as u32 } else { 0 }
}

/// A node's backdrop effects (background blurs and glass, in list order) for its `w` × `h` layer,
/// from `back`: its backdrop around the layer, `pad` pixels more each way. `shape` maps through
/// `xf` (local → layer pixels) at `scale` device pixels per unit. `None` when nothing shows.
#[allow(clippy::too_many_arguments)]
pub fn effects(n: &Node, shape: &Outline, back: &Pixmap, pad: u32, w: u32, h: u32, xf: Affine, scale: f64) -> Option<Pixmap> {
    let pad_i = i32::try_from(pad).ok()?;
    let mut out: Option<Pixmap> = None;
    let mut cov: Option<Pixmap> = None;
    for e in n.effects.iter().filter(|e| e.visible) {
        let layer = match &e.kind {
            EffectKind::BackgroundBlur(blur) if blur.max_radius() > 0.0 => {
                let mut b = back.clone();
                crate::blur::apply(&mut b, blur, (n.width, n.height), Affine::translate((f64::from(pad), f64::from(pad))) * xf, scale);
                let mut l = Pixmap::new(w, h)?;
                let copy = PixmapPaint { blend_mode: tiny_skia::BlendMode::Source, ..Default::default() };
                l.draw_pixmap(-pad_i, -pad_i, b.as_ref(), &copy, Transform::identity(), None);
                if cov.is_none() {
                    cov = spread_alpha(std::slice::from_ref(shape), 0.0, xf, w, h);
                }
                fx::mask_by(&mut l, cov.as_ref()?, false);
                l
            }
            EffectKind::Glass(g) => glass::glass(g, shape, back, pad, w, h, xf, scale)?,
            _ => continue,
        };
        match out.as_mut() {
            Some(o) => o.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None),
            None => out = Some(layer),
        }
    }
    out
}

/// Hide node `id` and everything painted after it (later siblings up its ancestry, the later
/// `roots` and the strokes its ancestors paint over their children): what is left is its backdrop
/// on the page made of `roots` (in z-order).
pub fn hide_above(doc: &mut Document, id: NodeId, roots: &[NodeId]) {
    let mut hide: Vec<NodeId> = vec![id];
    let mut cur = id;
    for _ in 0..=MAX_DEPTH {
        if let Some(i) = roots.iter().position(|r| *r == cur) {
            hide.extend(roots.iter().skip(i + 1));
            break;
        }
        let Some(parent) = doc.node(cur).and_then(|n| n.parent) else { break };
        if let Some(p) = doc.node(parent) {
            hide.extend(p.paint_order().skip_while(|c| *c != cur).skip(1));
        }
        if let Some(pn) = doc.node_mut(parent) {
            pn.strokes.clear();
        }
        cur = parent;
    }
    for h in hide {
        if let Some(n) = doc.node_mut(h) {
            n.visible = false;
        }
    }
}

/// What lies under node `id` when `roots` (in z-order) are drawn into `view`: every layer drawn
/// before it, its ancestors' own fills included (see [`hide_above`]), the way the renderer's
/// [`backdrop`] sees it.
pub fn render_beneath(doc: &Document, roots: &[NodeId], id: NodeId, view: &View) -> Option<Pixmap> {
    let mut beneath = doc.clone();
    hide_above(&mut beneath, id, roots);
    let mut pix = Pixmap::new(view.width.clamp(1, 16_384), view.height.clamp(1, 16_384))?;
    crate::render_nodes_into(&beneath, roots, view, &mut pix);
    Some(pix)
}

/// The shape a node's backdrop effects show through: its outline; a group is no outline of its
/// own, so the outlines of the layers in it, together.
pub fn outline(doc: &Document, n: &Node) -> Option<Outline> {
    match n.kind {
        NodeKind::Group => crate::subtree_outline(doc, n, 0).filter(|(p, _)| !p.elements().is_empty()),
        _ => crate::node_outline(doc, n),
    }
}

/// Where node `id`'s shape lies on its page (what its backdrop effects cover).
pub fn shape_bounds(doc: &Document, id: NodeId) -> Option<Rect> {
    let shape = outline(doc, doc.node(id)?)?;
    Some(figcraft_geom::transform_rect_bbox(doc.world_transform(id), figcraft_geom::Shape::bounding_box(&shape.0)))
}

/// Node `id`'s backdrop effects as `view`'s pixels (a view of its [`shape_bounds`]), drawn from a
/// render of what lies under it when `roots` are drawn ([`render_beneath`]): how exports, which
/// have no canvas to read, draw background blur and glass. Nothing lies outside `keep` (page
/// coordinates), when given.
pub fn effects_beneath(doc: &Document, roots: &[NodeId], id: NodeId, view: &View, keep: Option<Rect>) -> Option<Pixmap> {
    let n = doc.node(id)?;
    let shape = outline(doc, n)?;
    let pad = pad(n, view.scale);
    let p = f64::from(pad) / view.scale.max(1e-9);
    let around = View {
        origin: (view.origin.0 - p, view.origin.1 - p),
        scale: view.scale,
        width: view.width.saturating_add(pad * 2),
        height: view.height.saturating_add(pad * 2),
    };
    let mut back = render_beneath(doc, roots, id, &around)?;
    if let Some(k) = keep {
        let s = view.scale;
        let (ox, oy) = around.origin;
        keep_inside(&mut back, Rect::new((k.x0 - ox) * s, (k.y0 - oy) * s, (k.x1 - ox) * s, (k.y1 - oy) * s));
    }
    effects(n, &shape, &back, pad, view.width, view.height, view.transform() * doc.world_transform(id), view.scale)
}

/// Clear the pixels of `p` outside `r` (pixel coordinates).
fn keep_inside(p: &mut Pixmap, r: Rect) {
    let w = p.width() as usize;
    for (i, px) in p.data_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
        if !(x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1) {
            *px = [0; 4];
        }
    }
}
