//! The scene walk: nodes → krilla surface operations, in page space.

use std::collections::HashMap;

use figcraft_color::{BlendMode, Gradient, GradientKind, ImagePaint, ImageScaleMode, Paint, PaintKind, PatternPaint};
use figcraft_doc::{Document, EffectKind, MaskType as DocMaskType, Node, NodeId, NodeKind, StrokeAlign};
use figcraft_geom::{Affine, BezPath, Rect, Shape};

use crate::raster::Raster;
use krilla::geom::{Path, PathBuilder, Size, Transform};
use krilla::image::Image;
use krilla::mask::{Mask, MaskType};
use krilla::num::NormalizedF32;
use krilla::paint::{Fill, FillRule, LinearGradient, Pattern, RadialGradient, SpreadMethod, Stop};
use krilla::surface::Surface;

use crate::PdfOptions;
use crate::print::Ink;

/// Deepest node nesting drawn (the renderer's limit).
pub(crate) const MAX_DEPTH: usize = figcraft_render::MAX_EXPORT_DEPTH;
/// Largest side of any raster made for the PDF, in pixels.
pub(crate) const MAX_RASTER_SIDE: f64 = 4096.0;

pub(crate) struct Exporter<'a> {
    pub doc: &'a Document,
    pub opts: &'a PdfOptions,
    /// Page coordinates → PDF page coordinates for the page being drawn.
    pub page_origin: Affine,
    /// The page's area in document page coordinates.
    pub page_rect: Rect,
    /// The layers drawn on the current page (z-order): what background blurs see behind them.
    pub page_ids: Vec<NodeId>,
    /// Text hyperlinks on the current page: their areas (PDF page coordinates) and URLs.
    pub links: Vec<(Rect, String)>,
    /// Above zero while drawing into a soft mask (nothing there is visible or clickable).
    pub(crate) masking: usize,
    /// Above zero while drawing a pattern tile (links and backdrops belong to the page).
    pub(crate) tiling: usize,
    /// The page's frame and the bleed its fills and clip reach out to.
    pub bleed_root: Option<(NodeId, f64)>,
    pub(crate) fonts: HashMap<u32, Option<krilla::text::Font>>,
    images: HashMap<String, Option<(Image, u32, u32)>>,
}

pub(crate) fn to_xf(a: Affine) -> Transform {
    let c = a.as_coeffs();
    Transform::from_row(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32, c[4] as f32, c[5] as f32)
}

/// Finite and in a range f32 PDF numbers can hold.
pub(crate) fn affine_ok(a: Affine) -> bool {
    a.as_coeffs().iter().all(|v| v.is_finite() && v.abs() < 1e9)
}

/// kurbo path → krilla path (`None` when empty, degenerate or not finite).
pub(crate) fn to_path(p: &BezPath) -> Option<Path> {
    use figcraft_geom::kurbo::PathEl;
    if p.elements().is_empty() || !figcraft_pathops::finite(p) {
        return None;
    }
    let mut b = PathBuilder::new();
    for el in p.elements() {
        match *el {
            PathEl::MoveTo(a) => b.move_to(a.x as f32, a.y as f32),
            PathEl::LineTo(a) => b.line_to(a.x as f32, a.y as f32),
            PathEl::QuadTo(c, a) => b.quad_to(c.x as f32, c.y as f32, a.x as f32, a.y as f32),
            PathEl::CurveTo(c1, c2, a) => b.cubic_to(c1.x as f32, c1.y as f32, c2.x as f32, c2.y as f32, a.x as f32, a.y as f32),
            PathEl::ClosePath => b.close(),
        }
    }
    b.finish()
}

/// `v > min`, false for NaN.
pub(crate) fn above<T: PartialOrd>(v: T, min: T) -> bool {
    v > min
}

pub(crate) fn norm(v: f32) -> NormalizedF32 {
    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    NormalizedF32::new(v).unwrap_or(NormalizedF32::ONE)
}

/// The PDF blend mode for a non-normal blend (`None` = normal / pass through).
pub(crate) fn blend(b: BlendMode) -> Option<krilla::blend::BlendMode> {
    use krilla::blend::BlendMode as K;
    Some(match b {
        BlendMode::PassThrough | BlendMode::Normal => return None,
        BlendMode::Darken => K::Darken,
        BlendMode::Multiply => K::Multiply,
        // PDF has no plus-darker/plus-lighter: the nearest separable modes.
        BlendMode::PlusDarker => K::Multiply,
        BlendMode::ColorBurn => K::ColorBurn,
        BlendMode::Lighten => K::Lighten,
        BlendMode::Screen => K::Screen,
        BlendMode::PlusLighter => K::Screen,
        BlendMode::ColorDodge => K::ColorDodge,
        BlendMode::Overlay => K::Overlay,
        BlendMode::SoftLight => K::SoftLight,
        BlendMode::HardLight => K::HardLight,
        BlendMode::Difference => K::Difference,
        BlendMode::Exclusion => K::Exclusion,
        BlendMode::Hue => K::Hue,
        BlendMode::Saturation => K::Saturation,
        BlendMode::Color => K::Color,
        BlendMode::Luminosity => K::Luminosity,
    })
}

pub(crate) fn rule(r: figcraft_render::FillRule) -> FillRule {
    match r {
        figcraft_render::FillRule::EvenOdd => FillRule::EvenOdd,
        figcraft_render::FillRule::Winding => FillRule::NonZero,
    }
}

/// The renderer's stroke fill rule: even-odd ring paths for aligned strokes, else non-zero.
fn stroke_rule(n: &Node, closed: bool) -> FillRule {
    if n.stroke.sides.is_some() || (n.stroke.dashes.is_empty() && closed && n.stroke.align != StrokeAlign::Center) {
        FillRule::EvenOdd
    } else {
        FillRule::NonZero
    }
}

/// The gradient's unit frame in a w×h box: (0,0)→from, (1,0)→to, (0,1)→width handle.
fn handle_frame(g: &Gradient, w: f64, h: f64) -> Affine {
    let f = (g.from.0 * w, g.from.1 * h);
    let t = (g.to.0 * w, g.to.1 * h);
    let k = (g.width.0 * w, g.width.1 * h);
    Affine::new([t.0 - f.0, t.1 - f.1, k.0 - f.0, k.1 - f.1, f.0, f.1])
}

/// A paint PDF can express directly (solid, linear, radial) on a w×h box, in `ink`, with its fill
/// opacity.
pub(crate) fn vector_paint(p: &Paint, w: f64, h: f64, ink: Ink) -> Option<(krilla::paint::Paint, NormalizedF32)> {
    if !p.visible || !above(p.opacity, 0.0) {
        return None;
    }
    let op = p.opacity.clamp(0.0, 1.0);
    match &p.kind {
        PaintKind::Solid { color } => {
            let a = color.a * op;
            (a > 0.0).then(|| (ink.color(*color).into(), norm(a)))
        }
        PaintKind::Gradient(g) if matches!(g.kind, GradientKind::Linear | GradientKind::Radial) => {
            let frame = handle_frame(g, w.max(1e-6), h.max(1e-6));
            if !affine_ok(frame) || !above(frame.determinant().abs(), 1e-12) {
                return None;
            }
            let mut stops: Vec<Stop> = g
                .sorted_stops()
                .into_iter()
                .map(|s| Stop { offset: norm(s.position), color: ink.color(s.color), opacity: norm(s.color.a * op) })
                .collect();
            if stops.len() == 1
                && let Some(only) = stops.first().cloned()
            {
                stops = vec![Stop { offset: norm(0.0), ..only.clone() }, Stop { offset: norm(1.0), ..only }];
            }
            let transform = to_xf(frame);
            let paint: krilla::paint::Paint = match g.kind {
                GradientKind::Linear => {
                    LinearGradient { x1: 0.0, y1: 0.0, x2: 1.0, y2: 0.0, transform, spread_method: SpreadMethod::Pad, stops, anti_alias: false }
                        .into()
                }
                _ => RadialGradient {
                    fx: 0.0,
                    fy: 0.0,
                    fr: 0.0,
                    cx: 0.0,
                    cy: 0.0,
                    cr: 1.0,
                    transform,
                    spread_method: SpreadMethod::Pad,
                    stops,
                    anti_alias: false,
                }
                .into(),
            };
            Some((paint, NormalizedF32::ONE))
        }
        _ => None,
    }
}

enum Sniffed {
    Png,
    Jpeg,
    Gif,
    Webp,
    Other,
}

fn sniff(b: &[u8]) -> Sniffed {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        Sniffed::Png
    } else if b.starts_with(&[0xFF, 0xD8]) {
        Sniffed::Jpeg
    } else if b.starts_with(b"GIF") {
        Sniffed::Gif
    } else if b.len() > 12 && b.get(8..12) == Some(b"WEBP") {
        Sniffed::Webp
    } else {
        Sniffed::Other
    }
}

impl<'a> Exporter<'a> {
    pub(crate) fn new(doc: &'a Document, opts: &'a PdfOptions) -> Self {
        Self {
            doc,
            opts,
            page_origin: Affine::IDENTITY,
            page_rect: Rect::ZERO,
            page_ids: Vec::new(),
            links: Vec::new(),
            masking: 0,
            tiling: 0,
            bleed_root: None,
            fonts: HashMap::new(),
            images: HashMap::new(),
        }
    }

    /// The colour model of what's drawn now: CMYK when printing in it, but never in soft masks
    /// (they aren't printed).
    pub(crate) fn ink(&self) -> Ink {
        if self.opts.print.cmyk && self.masking == 0 { Ink::Cmyk } else { Ink::Rgb }
    }

    /// Draw node `id`; `parent_xf` maps its parent's space to PDF page space. `as_mask` draws a
    /// mask layer itself (into a soft mask).
    pub(crate) fn node(&mut self, s: &mut Surface, id: NodeId, parent_xf: Affine, depth: usize, as_mask: bool) {
        if depth > MAX_DEPTH {
            return;
        }
        let doc = self.doc;
        let Some(n) = doc.node(id) else { return };
        if !n.visible || !above(n.opacity, 0.0) || (n.is_mask && !as_mask) {
            return;
        }
        let xf = parent_xf * n.transform;
        if !affine_ok(xf) {
            return;
        }
        let fx: Vec<&EffectKind> = n.effects.iter().filter(|e| e.visible).map(|e| &e.kind).collect();
        let op = n.opacity.clamp(0.0, 1.0);
        // Background blur and glass: an image drawn from what lies under the node, under its
        // content (nothing in a soft mask is seen).
        let back =
            if !as_mask && self.masking == 0 && self.tiling == 0 && figcraft_render::backdrop::has_effects(n) { self.backdrop(id) } else { None };
        if fx.iter().any(|k| matches!(k, EffectKind::LayerBlur { .. } | EffectKind::Noise(_) | EffectKind::Texture(_))) {
            // Effects that change every pixel: the renderer draws the whole node.
            if let Some(b) = &back {
                self.place_raster(s, b, op, n.blend);
            }
            self.raster_node(s, id);
            return;
        }
        let shadows = if fx.iter().any(|k| matches!(k, EffectKind::DropShadow(_) | EffectKind::InnerShadow(_))) { self.shadows(id) } else { None };
        // Drop shadows go under the node, with its opacity.
        if let Some(sh) = &shadows {
            for (r, b) in &sh.drop {
                self.place_raster(s, r, op, *b);
            }
        }
        let nb = blend(n.blend);
        if let Some(b) = nb {
            s.push_blend_mode(b);
        }
        // A blending layer, or a group set to Normal, composites its content in a group of its own.
        let isolated = nb.is_some() || n.isolates_children();
        if isolated {
            s.push_isolated();
        }
        if op < 1.0 {
            s.push_opacity(norm(op));
        }
        if let Some(b) = &back {
            self.place_raster(s, b, 1.0, BlendMode::Normal);
        }
        self.content(s, n, xf, depth, shadows.as_ref().map_or(&[], |sh| sh.inner.as_slice()));
        if op < 1.0 {
            s.pop();
        }
        if isolated {
            s.pop();
        }
        if nb.is_some() {
            s.pop();
        }
    }

    /// Fills, `inner` shadows (over the fills, under the children and strokes), children
    /// (clipped), strokes and text of a node, without opacity/effects.
    fn content(&mut self, s: &mut Surface, n: &Node, xf: Affine, depth: usize, inner: &[Raster]) {
        let (w, h) = (n.width, n.height);
        match &n.kind {
            NodeKind::Page { .. } | NodeKind::Group => {
                self.children(s, n, xf, depth);
                // A group's inner shadows lie over its layers.
                self.place_inner(s, inner);
            }
            NodeKind::Text(_) => crate::text::draw_text(self, s, n, xf, inner),
            _ => {
                // The page's frame reaches out to the bleed (fills and clip; paints keep their box).
                let outline = match self.bleed_root {
                    Some((id, b)) if id == n.id && n.kind.is_frame_like() => {
                        Some((Rect::new(-b, -b, w + b, h + b).to_path(0.1), figcraft_render::FillRule::Winding))
                    }
                    _ => figcraft_render::node_outline(self.doc, n),
                };
                let is_bool = matches!(n.kind, NodeKind::BooleanOp { .. });
                let closed = figcraft_doc::geometry::is_closed(n) || is_bool;
                if let Some((path, r)) = &outline
                    && closed
                {
                    for p in &n.fills {
                        self.fill(s, path, rule(*r), p, w, h, xf);
                    }
                }
                self.place_inner(s, inner);
                if !is_bool && !n.children.is_empty() {
                    if n.clip && n.kind.is_frame_like() {
                        // Clip content: children outside the frame's outline are cut off (and
                        // nothing shows through a degenerate frame, like the renderer).
                        let clip = outline.as_ref().and_then(|(p, _)| {
                            let mut cp = p.clone();
                            cp.apply_affine(xf);
                            to_path(&cp)
                        });
                        if let Some(cp) = clip {
                            s.push_clip_path(&cp, &FillRule::NonZero);
                            self.children(s, n, xf, depth);
                            s.pop();
                        }
                    } else {
                        self.children(s, n, xf, depth);
                    }
                }
                if n.has_visible_stroke() {
                    let geom = if is_bool { outline.as_ref().map(|(p, _)| p.clone()) } else { figcraft_doc::geometry::stroke_geometry(n) };
                    if let Some(g) = geom {
                        let area = figcraft_render::draw::stroke_area(n, &g, closed);
                        for p in &n.strokes {
                            self.fill(s, &area, stroke_rule(n, closed), p, w, h, xf);
                        }
                    }
                }
            }
        }
    }

    fn children(&mut self, s: &mut Surface, n: &Node, xf: Affine, depth: usize) {
        let doc = self.doc;
        let kids = &n.children;
        for run in doc.paint_runs(n) {
            let Some(&c) = kids.get(run.start) else { continue };
            if doc.node(c).is_some_and(|cn| cn.is_mask && cn.visible) {
                // Everything above the mask (until the next mask) shows through its alpha.
                self.masked(s, c, kids.get(run.start + 1..run.end).unwrap_or(&[]), xf, depth);
            } else {
                self.node(s, c, xf, depth + 1, false);
            }
        }
    }

    /// `content` drawn through mask layer `mask_id`: its alpha or luminance (a soft mask drawn from
    /// the mask layer, so vector, gradient and image masks all work), or its outline (a clip path).
    fn masked(&mut self, s: &mut Surface, mask_id: NodeId, content: &[NodeId], xf: Affine, depth: usize) {
        let Some(m) = self.doc.node(mask_id).filter(|_| !content.is_empty()) else { return };
        let kind = match m.mask_type {
            DocMaskType::Vector => {
                // The outline, fully opaque; nothing shows through a mask without one.
                let mut plain = m.clone();
                plain.is_mask = false;
                let clip = figcraft_render::subtree_outline(self.doc, &plain, depth + 1).and_then(|(mut p, r)| {
                    p.apply_affine(xf * m.transform);
                    Some((to_path(&p)?, rule(r)))
                });
                if let Some((cp, r)) = clip {
                    s.push_clip_path(&cp, &r);
                    self.draw_all(s, content, xf, depth);
                    s.pop();
                }
                return;
            }
            DocMaskType::Luminance => MaskType::Luminosity,
            DocMaskType::Alpha => MaskType::Alpha,
        };
        let stream = {
            let mut sb = s.stream_builder();
            let mut ms = sb.surface();
            self.masking += 1;
            self.node(&mut ms, mask_id, xf, depth + 1, true);
            self.masking -= 1;
            ms.finish();
            sb.finish()
        };
        s.push_mask(Mask::new(stream, kind));
        self.draw_all(s, content, xf, depth);
        s.pop();
    }

    fn draw_all(&mut self, s: &mut Surface, ids: &[NodeId], xf: Affine, depth: usize) {
        for c in ids {
            self.node(s, *c, xf, depth + 1, false);
        }
    }

    /// Fill `path` (local space, mapped to the page by `xf`) with one paint on a w×h box.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fill(&mut self, s: &mut Surface, path: &BezPath, r: FillRule, p: &Paint, w: f64, h: f64, xf: Affine) {
        if !p.visible || !above(p.opacity, 0.0) {
            return;
        }
        let Some(kp) = to_path(path) else { return };
        let pb = blend(p.blend);
        if let Some(b) = pb {
            s.push_blend_mode(b);
        }
        s.push_transform(&to_xf(xf));
        match &p.kind {
            PaintKind::Image(img) => self.image_fill(s, &kp, r, img, p.opacity.clamp(0.0, 1.0), w, h),
            PaintKind::Pattern(pt) => {
                if !self.pattern_fill(s, &kp, r, pt, p.opacity.clamp(0.0, 1.0), w, h) {
                    self.raster_paint(s, path, &kp, r, p, w, h, xf);
                }
            }
            PaintKind::Gradient(g) if !matches!(g.kind, GradientKind::Linear | GradientKind::Radial) => {
                self.raster_paint(s, path, &kp, r, p, w, h, xf)
            }
            _ => {
                if let Some((paint, opacity)) = vector_paint(p, w, h, self.ink()) {
                    s.set_fill(Some(Fill { paint, opacity, rule: r }));
                    s.draw_path(&kp);
                }
            }
        }
        s.pop();
        if pb.is_some() {
            s.pop();
        }
    }

    /// The PDF image for an image paint and its pixel size (cached per image + adjustments).
    fn image(&mut self, img: &ImagePaint) -> Option<(Image, u32, u32)> {
        let ink = self.ink();
        let key = format!("{}|{:?}|{ink:?}", img.image, img.filters);
        if let Some(i) = self.images.get(&key) {
            return i.clone();
        }
        let data = self.doc.images.get(&img.image);
        // Decoding through the renderer checks the image is readable and applies adjustments.
        let made = figcraft_render::images::get(&img.image, data, &img.filters, self.doc.color_profile).map(|pm| {
            let (pw, ph) = (pm.width(), pm.height());
            // Unadjusted RGB images embed their original bytes (JPEG stays JPEG).
            let original = data.filter(|_| img.filters.is_identity() && ink == Ink::Rgb).and_then(|d| {
                let bytes = krilla::Data::from(d.bytes.clone());
                match sniff(&d.bytes) {
                    Sniffed::Png => Image::from_png(bytes, true).ok(),
                    Sniffed::Jpeg => Image::from_jpeg(bytes, true).ok(),
                    Sniffed::Gif => Image::from_gif(bytes, true).ok(),
                    Sniffed::Webp => Image::from_webp(bytes, true).ok(),
                    Sniffed::Other => None,
                }
            });
            let image = original.filter(|i| i.size() == (pw, ph)).unwrap_or_else(|| ink.bitmap(figcraft_render::to_rgba8(&pm), pw, ph));
            (image, pw, ph)
        });
        self.images.insert(key, made.clone());
        made
    }

    /// An image fill; the surface is in the node's local space.
    #[allow(clippy::too_many_arguments)]
    fn image_fill(&mut self, s: &mut Surface, kp: &Path, r: FillRule, img: &ImagePaint, op: f32, w: f64, h: f64) {
        let Some((image, iw, ih)) = self.image(img) else { return };
        let t = figcraft_render::paint::image_transform(img, f64::from(iw), f64::from(ih), w, h);
        let t = Affine::new([f64::from(t.sx), f64::from(t.ky), f64::from(t.kx), f64::from(t.sy), f64::from(t.tx), f64::from(t.ty)]);
        let Some(size) = Size::from_wh(iw as f32, ih as f32) else { return };
        if !affine_ok(t) || !above(t.determinant().abs(), 1e-12) {
            return;
        }
        if op < 1.0 {
            s.push_opacity(norm(op));
        }
        if img.scale_mode == ImageScaleMode::Tile {
            let stream = {
                let mut sb = s.stream_builder();
                let mut ps = sb.surface();
                ps.draw_image(image, size);
                ps.finish();
                sb.finish()
            };
            let pattern = Pattern { stream, transform: to_xf(t), width: iw as f32, height: ih as f32 };
            s.set_fill(Some(Fill { paint: pattern.into(), opacity: NormalizedF32::ONE, rule: r }));
            s.draw_path(kp);
        } else {
            s.push_clip_path(kp, &r);
            s.push_transform(&to_xf(t));
            s.draw_image(image, size);
            s.pop();
            s.pop();
        }
        if op < 1.0 {
            s.pop();
        }
    }

    /// A pattern fill as a PDF tiling pattern of the source's vectors; false (draw it as an image
    /// instead) when the source has effects, which the PDF draws as page-placed images.
    #[allow(clippy::too_many_arguments)]
    fn pattern_fill(&mut self, s: &mut Surface, kp: &Path, r: FillRule, pt: &PatternPaint, op: f32, w: f64, h: f64) -> bool {
        let doc = self.doc;
        let Some(src) = doc.node(NodeId(pt.source)).filter(|n| !n.is_page()) else { return true };
        if doc.subtree(src.id).iter().filter_map(|i| doc.node(*i)).any(|n| n.effects.iter().any(|e| e.visible)) {
            return false;
        }
        let Some(cell) = pt.cell(src.width, src.height, w, h) else { return true };
        let unrotated = src.transform.inverse();
        let stream = {
            let mut sb = s.stream_builder();
            let mut ps = sb.surface();
            // Links and backdrops belong to the page, not to a tile.
            self.tiling += 1;
            let _ = figcraft_render::pattern::nested(|| {
                for (tx, ty) in &cell.tiles {
                    self.node(&mut ps, src.id, Affine::translate((*tx, *ty)) * Affine::scale(cell.scale) * unrotated, 0, false);
                }
                Some(())
            });
            self.tiling -= 1;
            ps.finish();
            sb.finish()
        };
        if op < 1.0 {
            s.push_opacity(norm(op));
        }
        let pattern = Pattern { stream, transform: to_xf(Affine::translate(cell.origin)), width: cell.size.0 as f32, height: cell.size.1 as f32 };
        s.set_fill(Some(Fill { paint: pattern.into(), opacity: NormalizedF32::ONE, rule: r }));
        s.draw_path(kp);
        if op < 1.0 {
            s.pop();
        }
        true
    }

    /// Angular and diamond gradients, and patterns of sources with effects: an image of the paint
    /// over the path's bounds, clipped to it. The surface is in the node's local space; `xf` (local → page) sets the resolution.
    #[allow(clippy::too_many_arguments)]
    fn raster_paint(&mut self, s: &mut Surface, path: &BezPath, kp: &Path, r: FillRule, p: &Paint, w: f64, h: f64, xf: Affine) {
        let bb = path.bounding_box();
        if !(bb.width() > 0.0 && bb.height() > 0.0 && crate::rect_finite(bb)) {
            return;
        }
        let page_scale = xf.determinant().abs().sqrt();
        let mut k = self.raster_scale() * if page_scale.is_finite() && page_scale > 0.0 { page_scale } else { 1.0 };
        let side = bb.width().max(bb.height()) * k;
        if side > MAX_RASTER_SIDE {
            k *= MAX_RASTER_SIDE / side;
        }
        let (pw, ph) = ((bb.width() * k).ceil().max(1.0) as u32, (bb.height() * k).ceil().max(1.0) as u32);
        let Some(prep) = figcraft_render::paint::prepare(p, w, h, k, self.doc) else { return };
        let Some(mut pix) = tiny_skia::Pixmap::new(pw, ph) else { return };
        let paint = tiny_skia::Paint { shader: prep.shader(), anti_alias: false, ..Default::default() };
        let Some(rect) = tiny_skia::Rect::from_xywh(bb.x0 as f32, bb.y0 as f32, bb.width() as f32, bb.height() as f32) else { return };
        pix.fill_rect(rect, &paint, tiny_skia::Transform::from_row(k as f32, 0.0, 0.0, k as f32, (-bb.x0 * k) as f32, (-bb.y0 * k) as f32), None);
        let Some(size) = Size::from_wh(pw as f32, ph as f32) else { return };
        let image = self.ink().bitmap(figcraft_render::to_rgba8(&pix), pw, ph);
        s.push_clip_path(kp, &r);
        s.push_transform(&to_xf(Affine::translate((bb.x0, bb.y0)) * Affine::scale(1.0 / k)));
        s.draw_image(image, size);
        s.pop();
        s.pop();
    }

    pub(crate) fn raster_scale(&self) -> f64 {
        let k = self.opts.raster_scale;
        if k.is_finite() { k.clamp(0.25, 8.0) } else { 2.0 }
    }
}
