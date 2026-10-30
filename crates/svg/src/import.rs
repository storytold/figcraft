//! SVG import: usvg's normalised tree (CSS, `<use>`, units, inheritance and the like resolved) →
//! an editable frame.
//!
//! - Groups keep their `id` as their name and stay groups; the wrappers SVG needs for a transform,
//!   opacity or filter on a single element merge into that element.
//! - Transforms are baked into the layers (groups follow their children, like everywhere else):
//!   rectangles (with a uniform corner radius), ellipses and lines stay those shapes, rotated or
//!   flipped as needed; anything else, or anything skewed, becomes a vector.
//! - Gradients stay gradients (`gradientTransform`, `reflect`/`repeat`, `href` chains) and patterns
//!   become image fills ([`paint`]); clip paths
//!   become vector masks and masks luminance or alpha masks; filters become drop shadows, inner
//!   shadows and layer blurs ([`fx`]); `mix-blend-mode` and opacity carry over.
//! - Text becomes text layers ([`text`]); images become rectangles with an image fill, and an SVG
//!   image is imported in place.
//!
//! What can't be represented (filters other than shadows and blurs, rotated repeating patterns,
//! nested clip paths) is left out and reported in the import notes. Linked files are never read: only data URIs.

mod budget;
mod css_color;
mod fx;
mod paint;
mod shape;
mod text;

#[cfg(test)]
mod budget_tests;
#[cfg(test)]
mod css_color_tests;
#[cfg(test)]
mod fx_tests;
#[cfg(test)]
mod hostile_tests;
#[cfg(test)]
mod paint_tests;
#[cfg(test)]
mod shape_tests;
#[cfg(test)]
mod structure_tests;
#[cfg(test)]
mod tile_tests;

use std::collections::{BTreeSet, HashMap, HashSet};

use figcraft_color::{BlendMode, ImageScaleMode, Paint};
use figcraft_doc::{Document, Effect, MaskType, Node, NodeId, NodeKind, StrokeAlign, StrokeCap, StrokeJoin, VectorNetwork, WindingRule};
use figcraft_geom::{Affine, BezPath, Point, Rect, Shape};

use paint::Target;
use shape::{Prim, length_scale, place};

#[derive(Debug)]
pub struct ImportError(pub String);

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SVG import failed: {}", self.0)
    }
}

/// An SVG import: the frame holding it and notes on what couldn't be represented.
#[derive(Debug)]
pub struct Imported {
    pub frame: NodeId,
    pub notes: Vec<String>,
}

pub(crate) fn xf(t: usvg::Transform) -> Affine {
    Affine::new([f64::from(t.sx), f64::from(t.ky), f64::from(t.kx), f64::from(t.sy), f64::from(t.tx), f64::from(t.ty)])
}

pub(crate) fn to_bez(p: &usvg::tiny_skia_path::Path) -> BezPath {
    use usvg::tiny_skia_path::PathSegment::*;
    let pt = |p: usvg::tiny_skia_path::Point| (f64::from(p.x), f64::from(p.y));
    let mut b = BezPath::new();
    for s in p.segments() {
        match s {
            MoveTo(a) => b.move_to(pt(a)),
            LineTo(a) => b.line_to(pt(a)),
            QuadTo(c, a) => b.quad_to(pt(c), pt(a)),
            CubicTo(c1, c2, a) => b.curve_to(pt(c1), pt(c2), pt(a)),
            Close => b.close_path(),
        }
    }
    b
}

pub(crate) fn finite(a: Affine) -> bool {
    a.as_coeffs().iter().all(|v| v.is_finite())
}

const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 128;

pub(crate) struct Ctx<'a> {
    pub doc: &'a mut Document,
    count: usize,
    /// Groups made for SVG structure rather than a named `<g>`: merged into their only child.
    anonymous: HashSet<NodeId>,
    notes: BTreeSet<String>,
    fonts: Option<text::Fonts>,
    /// Patterns being drawn into tile images (inside one another).
    pattern_depth: usize,
    /// Tiles drawn so far, by pattern and scale (see [`paint`]'s `render_tile`).
    tiles: HashMap<(*const usvg::Pattern, u64), Option<paint::Tile>>,
}

impl<'a> Ctx<'a> {
    pub(crate) fn new(doc: &'a mut Document) -> Self {
        Ctx { doc, count: 0, anonymous: HashSet::new(), notes: BTreeSet::new(), fonts: None, pattern_depth: 0, tiles: HashMap::new() }
    }

    pub(crate) fn full(&self) -> bool {
        self.count >= MAX_NODES
    }

    pub(crate) fn note(&mut self, s: impl Into<String>) {
        self.notes.insert(s.into());
    }

    /// Insert `n` on top of `parent`'s children.
    pub(crate) fn add(&mut self, n: Node, parent: NodeId) -> Option<NodeId> {
        if self.full() {
            return None;
        }
        let id = self.doc.insert(n, parent, None).ok()?;
        self.count += 1;
        Some(id)
    }

    pub(crate) fn group(&mut self, parent: NodeId, name: &str, anonymous: bool) -> Option<NodeId> {
        let n = Node::new(self.doc.alloc_id(), NodeKind::Group, name, 0.0, 0.0);
        let id = self.add(n, parent)?;
        if anonymous {
            self.anonymous.insert(id);
        }
        Some(id)
    }
}

/// A layer name from an SVG `id`: `_xHH_` escapes (written for spaces and punctuation) decoded.
pub(crate) fn layer_name(id: &str, fallback: &str) -> String {
    if id.is_empty() {
        return fallback.to_string();
    }
    let mut out = String::new();
    let mut rest = id;
    while let Some(i) = rest.find("_x") {
        let (head, tail) = rest.split_at(i);
        out.push_str(head);
        let hex = tail.get(2..4).filter(|_| tail.get(4..5) == Some("_"));
        match hex.and_then(|h| u8::from_str_radix(h, 16).ok()).map(char::from) {
            Some(c) if !c.is_control() => {
                out.push(c);
                rest = tail.get(5..).unwrap_or("");
            }
            _ => {
                out.push_str("_x");
                rest = tail.get(2..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out
}

fn blend(b: usvg::BlendMode) -> Option<BlendMode> {
    use usvg::BlendMode as B;
    Some(match b {
        B::Normal => return None,
        B::Multiply => BlendMode::Multiply,
        B::Screen => BlendMode::Screen,
        B::Overlay => BlendMode::Overlay,
        B::Darken => BlendMode::Darken,
        B::Lighten => BlendMode::Lighten,
        B::ColorDodge => BlendMode::ColorDodge,
        B::ColorBurn => BlendMode::ColorBurn,
        B::HardLight => BlendMode::HardLight,
        B::SoftLight => BlendMode::SoftLight,
        B::Difference => BlendMode::Difference,
        B::Exclusion => BlendMode::Exclusion,
        B::Hue => BlendMode::Hue,
        B::Saturation => BlendMode::Saturation,
        B::Color => BlendMode::Color,
        B::Luminosity => BlendMode::Luminosity,
    })
}

/// Convert `g`'s children into `parent`; `base` maps `g`'s tree into the frame.
pub(crate) fn children(cx: &mut Ctx, g: &usvg::Group, parent: NodeId, base: Affine, depth: usize) {
    if depth > MAX_DEPTH {
        cx.note(format!("layers nested more than {MAX_DEPTH} deep were left out"));
        return;
    }
    for child in g.children() {
        if cx.full() {
            cx.note(format!("more than {MAX_NODES} layers: the rest was left out"));
            return;
        }
        match child {
            usvg::Node::Group(sub) => group(cx, sub, parent, base, depth + 1),
            usvg::Node::Path(p) => path(cx, p, parent, base),
            usvg::Node::Image(img) => image(cx, img, img.id(), parent, base, depth + 1),
            usvg::Node::Text(t) => text::text(cx, t, parent, base, depth + 1),
        }
    }
}

fn group(cx: &mut Ctx, g: &usvg::Group, parent: NodeId, base: Affine, depth: usize) {
    let world = base * xf(g.abs_transform());
    if depth > MAX_DEPTH || !finite(world) {
        return;
    }
    if g.mask().is_some_and(|m| !m.root().has_children()) {
        // An empty mask hides everything.
        return;
    }
    // `<image id>` arrives as a group (placing it) named by the id around an unnamed image.
    if let [usvg::Node::Image(img)] = g.children()
        && img.id().is_empty()
        && g.clip_path().is_none()
        && g.mask().is_none()
        && g.filters().is_empty()
        && g.opacity().get() >= 1.0
        && g.blend_mode() == usvg::BlendMode::Normal
    {
        image(cx, img, g.id(), parent, base, depth + 1);
        return;
    }
    let effects: Vec<Effect> = if g.filters().is_empty() {
        Vec::new()
    } else {
        fx::effects(g.filters(), world).unwrap_or_else(|| {
            let ids: Vec<&str> = g.filters().iter().map(|f| f.id()).collect();
            cx.note(format!("filter {} isn't a shadow or blur: the layer is drawn without it", ids.join(", ")));
            Vec::new()
        })
    };
    let clip = g.clip_path().filter(|c| !g.filters().is_empty() || !clip_is_noop(c, g, base));
    let mask = g.mask();
    let masked = clip.is_some() || mask.is_some();
    let name = layer_name(g.id(), if masked { "Mask group" } else { "Group" });
    let Some(outer) = cx.group(parent, &name, g.id().is_empty()) else { return };
    if let Some(n) = cx.doc.node_mut(outer) {
        n.opacity = g.opacity().get();
        n.blend = if let Some(b) = blend(g.blend_mode()) {
            b
        } else if g.isolate() {
            BlendMode::Normal
        } else {
            BlendMode::PassThrough
        };
    }
    let mut inner = outer;
    if let Some(c) = clip {
        mask_layer(cx, c.root(), inner, world * xf(c.transform()), MaskType::Vector, depth);
        if c.clip_path().is_some() {
            cx.note("a clip path clipped by another clips by the first only");
        }
        if mask.is_some()
            && let Some(id) = cx.group(inner, "Mask group", true)
        {
            inner = id;
        }
    }
    if let Some(m) = mask {
        let kind = if m.kind() == usvg::MaskType::Alpha { MaskType::Alpha } else { MaskType::Luminance };
        mask_layer(cx, m.root(), inner, world, kind, depth);
        if m.mask().is_some() {
            cx.note("a mask masked by another masks by the first only");
        }
    }
    // Filters apply before clipping and masking: on the content.
    let content = if masked && !effects.is_empty() { cx.group(inner, "Group", true).unwrap_or(inner) } else { inner };
    if let Some(n) = cx.doc.node_mut(content) {
        n.effects = effects;
    }
    children(cx, g, content, base, depth);
    finish(cx, content);
    if inner != content {
        finish(cx, inner);
    }
    if outer != inner {
        finish(cx, outer);
    }
}

/// A clip path or mask's content as the mask layer at the bottom of `parent`.
fn mask_layer(cx: &mut Ctx, root: &usvg::Group, parent: NodeId, base: Affine, kind: MaskType, depth: usize) {
    let Some(id) = cx.group(parent, if kind == MaskType::Vector { "Clip path" } else { "Mask" }, true) else { return };
    if let Some(n) = cx.doc.node_mut(id) {
        n.is_mask = true;
        n.mask_type = kind;
    }
    children(cx, root, id, base, depth + 1);
    finish(cx, id);
}

/// Whether clip path `c` on `g` clips nothing: one rectangle, upright, around all of `g`'s content.
fn clip_is_noop(c: &usvg::ClipPath, g: &usvg::Group, base: Affine) -> bool {
    let [usvg::Node::Path(p)] = c.root().children() else { return false };
    if c.clip_path().is_some() {
        return false;
    }
    let Some(Prim::Rect { rect, radius }) = shape::recognize(&to_bez(p.data()), true) else { return false };
    let cw = base * xf(g.abs_transform()) * xf(c.transform()) * xf(p.abs_transform());
    let [_, b, cc, ..] = cw.as_coeffs();
    if radius > 0.0 || b.abs() > 1e-9 || cc.abs() > 1e-9 {
        return false;
    }
    let clip = figcraft_geom::transform_rect_bbox(cw, rect).inflate(0.01, 0.01);
    let r = g.abs_stroke_bounding_box();
    let content =
        figcraft_geom::transform_rect_bbox(base, Rect::new(f64::from(r.left()), f64::from(r.top()), f64::from(r.right()), f64::from(r.bottom())));
    clip.contains(Point::new(content.x0, content.y0)) && clip.contains(Point::new(content.x1, content.y1))
}

/// Done filling group `id`: drop it when it holds nothing, merge an anonymous one into its only
/// child, else fit it to its children.
pub(crate) fn finish(cx: &mut Ctx, id: NodeId) {
    let Some(n) = cx.doc.node(id) else { return };
    if !matches!(n.kind, NodeKind::Group) {
        return;
    }
    if !n.children.iter().any(|c| cx.doc.node(*c).is_some_and(|k| !k.is_mask)) {
        let _ = cx.doc.remove(id);
        return;
    }
    if let &[child] = n.children.as_slice()
        && cx.anonymous.contains(&id)
        && merge_into_child(cx, id, child)
    {
        return;
    }
    let _ = cx.doc.fit_to_children(id);
}

/// Move anonymous group `id`'s opacity, blend, effects and mask role onto its only `child` and put
/// the child in its place. `false` (nothing changed) when they don't combine.
fn merge_into_child(cx: &mut Ctx, id: NodeId, child: NodeId) -> bool {
    let (Some(g), Some(c)) = (cx.doc.node(id), cx.doc.node(child)) else { return false };
    let plain = |b: BlendMode| matches!(b, BlendMode::Normal | BlendMode::PassThrough);
    if c.is_mask || (!g.effects.is_empty() && !c.effects.is_empty()) || (!plain(g.blend) && !plain(c.blend)) {
        return false;
    }
    let (opacity, blend, effects, is_mask, mask_type) = (g.opacity, g.blend, g.effects.clone(), g.is_mask, g.mask_type);
    let (Some(parent), Some(at)) = (g.parent, cx.doc.index_in_parent(id)) else { return false };
    if cx.doc.reparent(child, parent, Some(at)).is_err() {
        return false;
    }
    let _ = cx.doc.remove(id);
    cx.anonymous.remove(&id);
    if let Some(c) = cx.doc.node_mut(child) {
        c.opacity *= opacity;
        if !plain(blend) {
            c.blend = blend;
        }
        if !effects.is_empty() {
            c.effects = effects;
        }
        if is_mask {
            c.is_mask = true;
            c.mask_type = mask_type;
        }
    }
    true
}

fn path(cx: &mut Ctx, p: &usvg::Path, parent: NodeId, base: Affine) {
    let world = base * xf(p.abs_transform());
    if !p.is_visible() || !finite(world) {
        return;
    }
    let local = to_bez(p.data());
    let (fill, stroke) = (p.fill(), p.stroke());
    let shaped = shape::recognize(&local, stroke.is_none()).and_then(|prim| primitive(prim, world));
    let (kind, transform, w, h, radius) = match shaped {
        Some(s) => s,
        None => {
            let mut path = local;
            path.apply_affine(world);
            let bb = path.bounding_box();
            if ![bb.x0, bb.y0, bb.x1, bb.y1].iter().all(|v| v.is_finite()) {
                return;
            }
            path.apply_affine(Affine::translate((-bb.x0, -bb.y0)));
            let rule = if fill.is_some_and(|f| f.rule() == usvg::FillRule::EvenOdd) { WindingRule::EvenOdd } else { WindingRule::NonZero };
            let vector = |path: &BezPath, filled: bool| {
                let mut network = VectorNetwork::from_path(path);
                for r in &mut network.regions {
                    r.winding = rule;
                }
                if !filled {
                    network.regions.clear();
                }
                NodeKind::Vector { network }
            };
            let at = Affine::translate((bb.x0, bb.y0));
            // Fill closes every subpath; a stroke only those that close themselves. A filled
            // and stroked path with a really open subpath becomes a fill layer and a stroke layer.
            let tol = CLOSE_TOLERANCE * bb.width().max(bb.height());
            let (snapped, closed) = (closed_for_fill(&path, tol, false), closed_for_fill(&path, tol, true));
            if let (Some(_), Some(_)) = (fill, stroke)
                && snapped != closed
            {
                let fill_only =
                    Node::new(cx.doc.alloc_id(), vector(&closed, true), format!("{} fill", layer_name(p.id(), "Vector")), bb.width(), bb.height());
                let stroke_only = Node::new(cx.doc.alloc_id(), vector(&snapped, false), layer_name(p.id(), "Vector"), bb.width(), bb.height());
                let mut layers = [paint_path(cx, p, fill_only, at, world, true, false), paint_path(cx, p, stroke_only, at, world, false, true)];
                if p.paint_order() == usvg::PaintOrder::StrokeAndFill {
                    layers.reverse();
                }
                for n in layers {
                    cx.add(n, parent);
                }
                return;
            }
            (vector(if fill.is_some() { &closed } else { &path }, fill.is_some()), at, bb.width(), bb.height(), 0.0)
        }
    };
    let fallback = match kind {
        NodeKind::Rectangle => "Rectangle",
        NodeKind::Ellipse { .. } => "Ellipse",
        NodeKind::Line => "Line",
        _ => "Vector",
    };
    let mut n = Node::new(cx.doc.alloc_id(), kind, layer_name(p.id(), fallback), w, h);
    n.corners.radii = [radius; 4];
    let n = paint_path(cx, p, n, transform, world, true, true);
    cx.add(n, parent);
}

/// Layer `n` of path `p` placed at `transform`, with the path's fill and / or stroke.
fn paint_path(cx: &mut Ctx, p: &usvg::Path, mut n: Node, transform: Affine, world: Affine, with_fill: bool, with_stroke: bool) -> Node {
    n.transform = transform;
    let target = Target { to_node: transform.inverse() * world, size: (n.width, n.height) };
    n.fills = p.fill().filter(|_| with_fill).and_then(|f| paint::paint(cx, f.paint(), f.opacity().get(), target)).into_iter().collect();
    n.strokes.clear();
    if let Some(s) = p.stroke().filter(|_| with_stroke) {
        n.strokes = paint::paint(cx, s.paint(), s.opacity().get(), target).into_iter().collect();
        n.stroke = stroke_props(s, length_scale(world));
        // A stroke painted under the fill shows only its outer half.
        if p.paint_order() == usvg::PaintOrder::StrokeAndFill && !n.fills.is_empty() && figcraft_doc::geometry::is_closed(&n) {
            n.stroke.align = StrokeAlign::Outside;
            n.stroke.weight /= 2.0;
        }
    }
    n
}

/// How far (as a share of the path's larger side) a subpath may end from its start and still be
/// closed onto it: exporters that round coordinates leave closed outlines this far apart.
const CLOSE_TOLERANCE: f64 = 1e-3;

/// `path` with the subpaths that end within `tol` of their start closed onto it and, with `all`
/// (SVG fills every subpath as if closed), every other subpath that can enclose an area (two or
/// more segments, or one curve) closed with a straight segment.
fn closed_for_fill(path: &BezPath, tol: f64, all: bool) -> BezPath {
    use figcraft_geom::kurbo::PathEl;
    let mut out = BezPath::new();
    let mut sub: Vec<PathEl> = Vec::new();
    let flush = |sub: &mut Vec<PathEl>, out: &mut BezPath| {
        let start = match sub.first() {
            Some(PathEl::MoveTo(p)) => *p,
            _ => {
                sub.clear();
                return;
            }
        };
        let open = !matches!(sub.last(), Some(PathEl::ClosePath));
        let segments = sub.len() - 1;
        let curve = matches!(sub.last(), Some(PathEl::QuadTo(..) | PathEl::CurveTo(..)));
        if open && (segments >= 2 || curve) {
            let near = sub.last().and_then(|e| e.end_point()).is_some_and(|end| (end - start).hypot() <= tol);
            if near && let Some(last) = sub.last_mut() {
                match last {
                    PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => *p = start,
                    PathEl::MoveTo(_) | PathEl::ClosePath => {}
                }
            }
            if near || all {
                sub.push(PathEl::ClosePath);
            }
        }
        for e in sub.drain(..) {
            out.push(e);
        }
    };
    for e in path.elements() {
        if matches!(e, PathEl::MoveTo(_)) {
            flush(&mut sub, &mut out);
        }
        sub.push(*e);
    }
    flush(&mut sub, &mut out);
    out
}

/// A recognised primitive placed by `world`: its kind, transform, size and corner radius. `None`
/// when `world` would skew it (or stretch its rounded corners).
fn primitive(prim: Prim, world: Affine) -> Option<(NodeKind, Affine, f64, f64, f64)> {
    match prim {
        Prim::Rect { rect, radius } => {
            let (t, w, h) = place(world, rect.origin(), rect.width(), rect.height())?;
            let (fx, fy) = (w / rect.width(), h / rect.height());
            if radius > 0.0 && (fx - fy).abs() > 1e-6 * fx.max(fy) {
                return None;
            }
            Some((NodeKind::Rectangle, t, w, h, radius * fx))
        }
        Prim::Ellipse(rect) => {
            let (t, w, h) = place(world, rect.origin(), rect.width(), rect.height())?;
            Some((NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, t, w, h, 0.0))
        }
        Prim::Line(a, b) => {
            let (a, b) = (world * a, world * b);
            let d = b - a;
            let len = d.hypot();
            (len > 1e-9 && len.is_finite()).then(|| (NodeKind::Line, Affine::translate(a.to_vec2()) * Affine::rotate(d.atan2()), len, 0.0, 0.0))
        }
    }
}

pub(crate) fn stroke_props(s: &usvg::Stroke, k: f64) -> figcraft_doc::StrokeProps {
    let cap = match s.linecap() {
        usvg::LineCap::Round => StrokeCap::Round,
        usvg::LineCap::Square => StrokeCap::Square,
        usvg::LineCap::Butt => StrokeCap::None,
    };
    let mut dashes: Vec<f64> = s.dasharray().unwrap_or(&[]).iter().map(|v| f64::from(*v) * k).collect();
    if !dashes.is_empty() {
        let offset = f64::from(s.dashoffset()) * k;
        // Half a dash in is how a line starts by itself (and how export writes one).
        if dashes.first().is_none_or(|d| (d / 2.0 - offset).abs() > 1e-3) {
            dashes = dash_offset(&dashes, offset, cap == StrokeCap::None).unwrap_or(dashes);
        }
    }
    figcraft_doc::StrokeProps {
        weight: f64::from(s.width().get()) * k,
        align: StrokeAlign::Center,
        cap_start: cap,
        cap_end: cap,
        join: match s.linejoin() {
            usvg::LineJoin::Round => StrokeJoin::Round,
            usvg::LineJoin::Bevel => StrokeJoin::Bevel,
            _ => StrokeJoin::Miter,
        },
        miter_limit: f64::from(s.miterlimit().get()),
        dashes,
        ..Default::default()
    }
}

/// The dash pattern started `offset` into it (dash patterns have no offset of their own). `None`
/// when it can't be expressed: an offset into a gap needs a zero-length dash first, which only
/// draws nothing with butt caps.
pub(crate) fn dash_offset(dashes: &[f64], offset: f64, butt: bool) -> Option<Vec<f64>> {
    let total: f64 = dashes.iter().sum();
    if !(total > 0.0 && offset.is_finite()) || dashes.len() % 2 == 1 || dashes.iter().any(|d| *d < 0.0) {
        return None;
    }
    let mut o = offset.rem_euclid(total);
    if o < 1e-9 {
        return Some(dashes.to_vec());
    }
    for (i, d) in dashes.iter().enumerate() {
        if o < *d {
            let mut out = Vec::with_capacity(dashes.len() + 2);
            if i % 2 == 1 {
                if !butt {
                    return None;
                }
                out.push(0.0);
            }
            out.push(d - o);
            out.extend(dashes.get(i + 1..).unwrap_or(&[]));
            out.extend(dashes.get(..i).unwrap_or(&[]));
            out.push(o);
            if out.len() % 2 == 1 {
                out.push(0.0);
            }
            return Some(out);
        }
        o -= d;
    }
    None
}

fn image(cx: &mut Ctx, img: &usvg::Image, id: &str, parent: NodeId, base: Affine, depth: usize) {
    let world = base * xf(img.abs_transform());
    if !img.is_visible() || !finite(world) {
        return;
    }
    let bytes = match img.kind() {
        usvg::ImageKind::PNG(b) | usvg::ImageKind::JPEG(b) | usvg::ImageKind::GIF(b) | usvg::ImageKind::WEBP(b) => b.clone(),
        usvg::ImageKind::SVG(tree) => {
            // A nested SVG document: imported in place.
            if let Some(g) = cx.group(parent, &layer_name(id, "Image"), id.is_empty()) {
                children(cx, tree.root(), g, world, depth + 1);
                finish(cx, g);
            }
            return;
        }
    };
    let size = img.size();
    let (iw, ih) = (f64::from(size.width()), f64::from(size.height()));
    let (transform, w, h) = place(world, Point::ZERO, iw, ih).unwrap_or((world, iw, ih));
    let hash = add_image(cx.doc, bytes, iw, ih);
    let mut n = Node::new(cx.doc.alloc_id(), NodeKind::Rectangle, layer_name(id, "Image"), w, h);
    n.transform = transform;
    n.fills = vec![Paint::image(hash, ImageScaleMode::Fill)];
    cx.add(n, parent);
}

/// Store image `bytes` (`w × h` pixels) in `doc`; returns its key.
pub(crate) fn add_image(doc: &mut Document, bytes: std::sync::Arc<Vec<u8>>, w: f64, h: f64) -> String {
    let hash = simple_hash(&bytes);
    doc.images.insert(hash.clone(), figcraft_doc::ImageData { bytes, width: w.round().max(1.0) as u32, height: h.round().max(1.0) as u32 });
    hash
}

fn simple_hash(b: &[u8]) -> String {
    // FNV-1a 128 (two lanes); the format crate re-keys by sha256 on save if needed.
    let mut h1: u64 = 0xcbf2_9ce4_8422_2325;
    let mut h2: u64 = 0x8422_2325_cbf2_9ce4;
    for (i, x) in b.iter().enumerate() {
        h1 ^= u64::from(*x);
        h1 = h1.wrapping_mul(0x0100_0000_01b3);
        h2 ^= u64::from(*x).wrapping_add(i as u64);
        h2 = h2.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h1:016x}{h2:016x}")
}

/// Deepest element nesting the parser is given: deeper documents would exhaust the stack.
const MAX_NESTING: usize = 512;

/// How deep elements nest in `data`: a scan of its tags (comments, CDATA, declarations and quoted
/// attribute values skipped) that stops once past `limit`.
pub(crate) fn nesting(data: &[u8], limit: usize) -> usize {
    let find = |from: usize, pat: &[u8]| -> usize {
        data.get(from..).and_then(|r| r.windows(pat.len()).position(|w| w == pat)).map_or(data.len(), |p| from + p + pat.len())
    };
    let (mut depth, mut max, mut i) = (0usize, 0usize, 0usize);
    while let Some(off) = data.get(i..).and_then(|r| r.iter().position(|b| *b == b'<')) {
        i += off;
        let rest = data.get(i..).unwrap_or(&[]);
        if rest.starts_with(b"<!--") {
            i = find(i + 4, b"-->");
        } else if rest.starts_with(b"<![CDATA[") {
            i = find(i + 9, b"]]>");
        } else if rest.starts_with(b"<?") || rest.starts_with(b"<!") {
            i = find(i + 2, b">");
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            i = find(i + 2, b">");
        } else {
            // A start tag: to its `>` outside quotes; `/>` closes it.
            let (mut quote, mut j, mut closed) = (None, i + 1, false);
            while let Some(&c) = data.get(j) {
                match quote {
                    Some(q) if c == q => quote = None,
                    Some(_) => {}
                    None if c == b'"' || c == b'\'' => quote = Some(c),
                    None if c == b'>' => {
                        closed = data.get(j.wrapping_sub(1)) == Some(&b'/');
                        break;
                    }
                    None => {}
                }
                j += 1;
            }
            if !closed {
                depth += 1;
                max = max.max(depth);
                if max > limit {
                    return max;
                }
            }
            i = j + 1;
        }
    }
    max
}

/// Whether `data` nests elements more than [`MAX_NESTING`] deep: the byte scan first, then the
/// parsed tree (walked without recursion), which also counts what DTD entity references expand to.
pub(crate) fn too_deep(data: &[u8]) -> bool {
    if nesting(data, MAX_NESTING) > MAX_NESTING {
        return true;
    }
    let Ok(text) = std::str::from_utf8(data) else { return false };
    let opt = usvg::roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let Ok(xml) = usvg::roxmltree::Document::parse_with_options(text, opt) else { return false };
    // Depth-first by sibling and parent links; the root is depth 0, each child one deeper.
    let (mut node, mut depth) = (xml.root(), 0usize);
    loop {
        if let Some(child) = node.first_child() {
            node = child;
            depth += 1;
            if depth > MAX_NESTING {
                return true;
            }
            continue;
        }
        loop {
            if let Some(next) = node.next_sibling() {
                node = next;
                break;
            }
            match node.parent() {
                Some(up) if depth > 0 => {
                    node = up;
                    depth -= 1;
                }
                _ => return false,
            }
        }
    }
}

fn options(data: &[u8], limits: budget::Limits) -> usvg::Options<'static> {
    // Only data URIs, within budgets: an opened or pasted SVG never pulls other files off the disk
    // and never expands without limit.
    let mut o = usvg::Options { image_href_resolver: budget::resolver(limits), ..Default::default() };
    if data.windows(4).any(|w| w == b"text") {
        text::prepare(&mut o);
    }
    o
}

/// Length of the `<!...>` declaration at the start of `s`, skipping `>` inside quotes and an
/// internal subset (`[ ... ]`).
fn doctype_end(s: &str) -> Option<usize> {
    let mut quote = None;
    let mut depth = 0u32;
    let mut skip_to = 0;
    for (i, c) in s.char_indices() {
        if i < skip_to {
            continue;
        }
        if quote.is_none() && depth > 0 {
            // Comments and processing instructions in the subset may hold quotes and brackets.
            let rest = &s[i..];
            let close = if rest.starts_with("<!--") {
                Some("-->")
            } else if rest.starts_with("<?") {
                Some("?>")
            } else {
                None
            };
            if let Some(close) = close {
                let len = rest.find(close)? + close.len();
                skip_to = i + len;
                continue;
            }
        }
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '[') => depth += 1,
            (None, ']') => depth = depth.saturating_sub(1),
            (None, '>') if depth == 0 => return Some(i + 1),
            _ => {}
        }
    }
    None
}

/// Whether `text` (from the clipboard, say) is an SVG document: `<svg` after an optional byte
/// order mark, XML declaration, comments, doctype and whitespace.
pub fn looks_like_svg(text: &str) -> bool {
    let mut s = text.trim_start_matches('\u{feff}').trim_start();
    for _ in 0..64 {
        if s.starts_with("<svg") {
            return true;
        }
        let end = if s.starts_with("<?") {
            s.find("?>").map(|i| i + 2)
        } else if s.starts_with("<!--") {
            s.find("-->").map(|i| i + 3)
        } else if s.starts_with("<!") {
            doctype_end(s)
        } else {
            None
        };
        match end.and_then(|e| s.get(e..)) {
            Some(rest) => s = rest.trim_start(),
            None => return false,
        }
    }
    false
}

/// Import SVG bytes as a new frame under `parent` at `(x, y)`. Returns the frame id.
pub fn import_svg(doc: &mut Document, data: &[u8], parent: NodeId, x: f64, y: f64, name: &str) -> Result<NodeId, ImportError> {
    import_svg_with_notes(doc, data, parent, x, y, name).map(|i| i.frame)
}

/// [`import_svg`], with notes on what couldn't be represented.
pub fn import_svg_with_notes(doc: &mut Document, data: &[u8], parent: NodeId, x: f64, y: f64, name: &str) -> Result<Imported, ImportError> {
    import_within(doc, data, parent, x, y, name, budget::Limits::default())
}

fn import_within(
    doc: &mut Document,
    data: &[u8],
    parent: NodeId,
    x: f64,
    y: f64,
    name: &str,
    limits: budget::Limits,
) -> Result<Imported, ImportError> {
    // SVGZ is unpacked here, within a budget, so every check below sees the real document.
    let data = budget::document(data, &limits).map_err(ImportError)?;
    if too_deep(&data) {
        return Err(ImportError(format!("elements nested more than {MAX_NESTING} deep")));
    }
    // usvg reads neither `color()` nor `color-mix()`: they are resolved first.
    let (data, color_notes) = match css_color::resolve(&data, doc.color_profile) {
        Some(resolved) => resolved,
        None => (data, Vec::new()),
    };
    let tree = usvg::Tree::from_data(&data, &options(&data, limits)).map_err(|e| ImportError(e.to_string()))?;
    let size = tree.size();
    let frame =
        doc.create(parent, NodeKind::Frame, name, x, y, f64::from(size.width()), f64::from(size.height())).map_err(|e| ImportError(e.to_string()))?;
    if let Some(f) = doc.node_mut(frame) {
        f.fills.clear();
    }
    let mut cx = Ctx::new(doc);
    color_notes.into_iter().for_each(|n| cx.note(n));
    children(&mut cx, tree.root(), frame, Affine::IDENTITY, 0);
    Ok(Imported { frame, notes: cx.notes.into_iter().collect() })
}
