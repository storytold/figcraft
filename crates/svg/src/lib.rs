//! SVG export and import.
//!
//! Export writes one `<svg>` sized to the exported nodes' bounds. Shapes become `<path>`s with
//! their fills (solid, linear/radial gradients; angular and diamond fall back to their first
//! colour... or a raster pattern), strokes are written as filled outlines so Inside/Outside
//! alignment survives exactly, text is outlined, images are embedded as data URIs, and shadows
//! and blurs become SVG filters. Shadow spread offsets the outline like the renderer (the shape
//! stroked twice the spread wide with mitred joins, cut away with a mask to shrink); inner shadows
//! are masked, blurred rectangles clipped to the shape, over the fills and under the strokes.
//!
//! What SVG filters can't match is drawn by the renderer and embedded as PNG images, so it looks
//! like the canvas: noise grains over the vector layer (crisp, blended with the noise's blend
//! mode), background blur and glass from a render of what lies under the layer among the exported
//! layers, and layers with texture or a progressive layer blur whole.
//!
//! Import (via usvg) turns an SVG file into an editable frame: groups, shapes, text, gradients,
//! masks and effects (see [`import_svg_with_notes`]).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod import;
pub use import::{ImportError, Imported, import_svg, import_svg_with_notes, looks_like_svg};

use base64::Engine as _;
use craft_svg::write::{SVG_NS, SvgWriter, Tag};
#[cfg(test)]
use craft_xml::esc_attr as esc;
use craft_xml::is_xml_char as xml_char;
use figcraft_color::{Color, GradientKind, Paint, PaintKind};
use figcraft_doc::{Document, EffectKind, Node, NodeId, NodeKind, SPREAD_MITER_LIMIT, Shadow};
use figcraft_geom::{Affine, BezPath, Rect};
use figcraft_render::shadow::Outline;
use figcraft_render::{Alone, ExportDepthError, MAX_EXPORT_DEPTH, Pixmap, View, backdrop, check_export_depth};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SvgError {
    #[error("nothing to export")]
    Empty,
    #[error(transparent)]
    Depth(#[from] ExportDepthError),
}

/// Device pixels per page unit of embedded renders (blurs, glass, texture).
const RASTER_SCALE: f64 = 2.0;
/// Largest side of an embedded render, in pixels.
const MAX_RASTER_SIDE: f64 = 4096.0;

#[derive(Clone, Debug)]
pub struct SvgOptions {
    /// Layer names as `id` attributes (unique in the file: repeats get `_2`, `_3`…).
    pub include_ids: bool,
    /// Text as outlines; off: `<text>` elements (selectable, drawn with the reader's fonts).
    pub outline_text: bool,
    /// Strokes as `stroke` attributes where that's exact (uniform weight; centred, or inside and
    /// outside on boxes and ellipses); off: every stroke as a filled outline.
    pub simplify_stroke: bool,
    /// The page area the SVG covers (default: the exported layers' export bounds).
    pub bounds: Option<Rect>,
}

impl Default for SvgOptions {
    fn default() -> Self {
        Self { include_ids: false, outline_text: true, simplify_stroke: true, bounds: None }
    }
}

struct NodePaint<'a> {
    node: &'a Node,
    xf: Affine,
    depth: usize,
    paint_clip: Option<String>,
    kid_clip: Option<String>,
    inherited: Option<String>,
    outline: Option<(BezPath, figcraft_render::FillRule)>,
    grains: String,
    size: (f64, f64),
}

struct W<'a> {
    doc: &'a Document,
    /// The exported layers, in z-order (what backdrops see).
    roots: &'a [NodeId],
    /// Page coordinates → SVG coordinates.
    page: Affine,
    defs: SvgWriter,
    body: SvgWriter,
    next: u32,
    opts: &'a SvgOptions,
    /// The mask layer being drawn as a mask's content (drawn like any layer, not skipped).
    mask: Option<NodeId>,
    /// The clip path of the clipping frames around the layers being written. A clipped group
    /// isolates what it holds (it is a stacking context), so clips go on what blends instead: a
    /// layer that isolates anyway, or each paint and child of a plain group.
    clip: Option<String>,
    /// `id` attributes written so far.
    used_ids: std::collections::BTreeSet<String>,
    /// The next `_k` suffix to try per base id, so repeated names don't rescan from `_2`.
    next_suffix: std::collections::HashMap<String, usize>,
    /// The `<image>` written to `defs` per image key, so paints of one image share its bytes.
    images: std::collections::HashMap<String, String>,
}

fn clip_url(clip: &str) -> String {
    format!("url(#{clip})")
}

fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let r = (v * 1000.0).round() / 1000.0;
    let s = format!("{r}");
    if s == "-0" { "0".into() } else { s }
}

fn path_d(p: &BezPath) -> String {
    craft_svg::write::path_data_with_format(p, num)
}

struct DashPiece {
    d: String,
    dasharray: Option<String>,
    dashoffset: Option<String>,
}

/// A stroke's path as `d` values, each with its dash attributes. A dashed one takes a `<path>` for
/// each subpath, as each fits the pattern to its own length (see
/// [`figcraft_pathops::width::line_dashes`]), so a viewer draws what the canvas does.
fn dashed_pieces(path: BezPath, k: &figcraft_doc::StrokeProps) -> Vec<DashPiece> {
    let list = |v: &[f64]| v.iter().map(|d| num(*d)).collect::<Vec<_>>().join(" ");
    let Some(pattern) = figcraft_pathops::width::dash_pattern(k) else {
        return vec![DashPiece { d: path_d(&path), dasharray: None, dashoffset: None }];
    };
    let piece = |sub: BezPath| {
        let (dashes, offset) = figcraft_pathops::width::subpath_dashes(&sub, &pattern);
        DashPiece { d: path_d(&sub), dasharray: Some(list(&dashes)), dashoffset: (offset > 0.0).then(|| num(offset)) }
    };
    figcraft_geom::split_subpaths(&path).into_iter().map(piece).collect()
}

/// A matrix scale/shear coefficient to 6 significant digits (unlike [`num`], small values survive).
fn coeff(v: f64) -> String {
    if !v.is_finite() || v == 0.0 {
        return "0".into();
    }
    let digits = 5 - v.abs().log10().floor() as i32;
    let p = 10f64.powi(digits);
    let r = (v * p).round() / p;
    format!("{r}")
}

fn matrix(a: Affine) -> String {
    let c = a.as_coeffs();
    // Snap float noise (e.g. 6e-17 from a 90 degree rotation) to zero, relative to the largest term.
    let eps = c[..4].iter().fold(0.0f64, |m, v| m.max(v.abs())) * 1e-12;
    let eps = eps.max(1e-12);
    let k = |v: f64| if v.abs() <= eps { "0".to_string() } else { coeff(v) };
    format!("matrix({} {} {} {} {} {})", k(c[0]), k(c[1]), k(c[2]), k(c[3]), num(c[4]), num(c[5]))
}

/// A view of page area `b` at `k` device pixels per unit (fewer when that makes it larger than
/// [`MAX_RASTER_SIDE`] a side).
fn raster_view(b: Rect, k: f64) -> Option<View> {
    if !(b.width() > 0.0 && b.height() > 0.0 && [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) && k.is_finite() && k > 0.0) {
        return None;
    }
    let side = b.width().max(b.height()) * k;
    let k = if side > MAX_RASTER_SIDE { k * MAX_RASTER_SIDE / side } else { k };
    let (w, h) = ((b.width() * k).ceil().max(1.0), (b.height() * k).ceil().max(1.0));
    Some(View { origin: (b.x0, b.y0), scale: k, width: w as u32, height: h as u32 })
}

/// CSS `mix-blend-mode` value for a blend mode other than normal (else `None`).
fn blend_css(b: figcraft_color::BlendMode) -> Option<&'static str> {
    if matches!(b, figcraft_color::BlendMode::Normal | figcraft_color::BlendMode::PassThrough) { None } else { Some(b.css()) }
}

fn hex(c: Color) -> String {
    format!("#{}", c.to_hex())
}

struct PaintAttrs {
    paint: String,
    opacity: Option<String>,
    blend: Option<&'static str>,
}

impl PaintAttrs {
    fn apply_fill<'w>(&self, tag: Tag<'w>) -> Tag<'w> {
        tag.attr("fill", &self.paint)
            .attr_opt("fill-opacity", self.opacity.as_deref())
            .attr_opt("style", self.blend.map(|b| format!("mix-blend-mode:{b}")))
    }

    fn apply_stroke<'w>(&self, tag: Tag<'w>) -> Tag<'w> {
        tag.attr("stroke", &self.paint)
            .attr_opt("stroke-opacity", self.opacity.as_deref())
            .attr_opt("style", self.blend.map(|b| format!("mix-blend-mode:{b}")))
    }
}

impl W<'_> {
    fn id(&mut self, prefix: &str) -> String {
        loop {
            self.next += 1;
            let id = format!("{prefix}{}", self.next);
            if self.used_ids.insert(id.clone()) {
                return id;
            }
        }
    }

    /// Fill attributes for a paint on a w×h box; `None` when it paints nothing.
    fn paint_attr(&mut self, p: &Paint, w: f64, h: f64) -> Option<PaintAttrs> {
        if !p.visible || p.opacity <= 0.0 {
            return None;
        }
        let blend = blend_css(p.blend);
        match &p.kind {
            PaintKind::Solid { color } => {
                let a = color.a * p.opacity;
                Some(PaintAttrs { paint: hex(*color), opacity: (a < 0.999).then(|| num(f64::from(a))), blend })
            }
            PaintKind::Gradient(g) => {
                let gid = self.id("paint");
                let (f, t, k) = ((g.from.0 * w, g.from.1 * h), (g.to.0 * w, g.to.1 * h), (g.width.0 * w, g.width.1 * h));
                match g.kind {
                    GradientKind::Linear => {
                        self.defs
                            .tag("linearGradient")
                            .attr("id", &gid)
                            .attr("x1", num(f.0))
                            .attr("y1", num(f.1))
                            .attr("x2", num(t.0))
                            .attr("y2", num(t.1))
                            .attr("gradientUnits", "userSpaceOnUse")
                            .open();
                    }
                    _ => {
                        // Unit circle mapped onto the handle ellipse.
                        let m = Affine::new([t.0 - f.0, t.1 - f.1, k.0 - f.0, k.1 - f.1, f.0, f.1]);
                        self.defs
                            .tag("radialGradient")
                            .attr("id", &gid)
                            .attr("cx", "0")
                            .attr("cy", "0")
                            .attr("r", "1")
                            .attr("gradientUnits", "userSpaceOnUse")
                            .attr("gradientTransform", matrix(m))
                            .open();
                    }
                }
                for s in g.sorted_stops() {
                    let a = s.color.a * p.opacity;
                    self.defs
                        .tag("stop")
                        .attr("offset", num(f64::from(s.position)))
                        .attr("stop-color", hex(s.color))
                        .attr_opt("stop-opacity", (a < 0.999).then(|| num(f64::from(a))))
                        .empty();
                }
                self.defs.close();
                Some(PaintAttrs { paint: clip_url(&gid), opacity: None, blend })
            }
            PaintKind::Image(img) => {
                let data = self.doc.images.get(&img.image)?;
                let pid = self.id("pattern");
                let iid = match self.images.get(&img.image) {
                    Some(iid) => iid.clone(),
                    None => {
                        let iid = self.id("image");
                        // Formats SVG viewers don't show (TIFF, BMP) go in as PNG.
                        let png;
                        let (mime, bytes) = match sniff_mime(&data.bytes) {
                            "application/octet-stream" => {
                                png = figcraft_render::images::decode(&data.bytes).and_then(|p| figcraft_render::encode_png(&p).ok())?;
                                ("image/png", png.as_slice())
                            }
                            m => (m, data.bytes.as_slice()),
                        };
                        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                        self.defs
                            .tag("image")
                            .attr("id", &iid)
                            .attr("width", data.width)
                            .attr("height", data.height)
                            .attr("href", format_args!("data:{mime};base64,{b64}"))
                            .empty();
                        self.images.insert(img.image.clone(), iid.clone());
                        iid
                    }
                };
                let t = figcraft_render::paint::image_transform(img, f64::from(data.width), f64::from(data.height), w, h);
                let m = Affine::new([f64::from(t.sx), f64::from(t.ky), f64::from(t.kx), f64::from(t.sy), f64::from(t.tx), f64::from(t.ty)]);
                let (pw, ph) = if img.scale_mode == figcraft_color::ImageScaleMode::Tile {
                    (f64::from(data.width), f64::from(data.height))
                } else {
                    (f64::from(data.width) * 4.0, f64::from(data.height) * 4.0)
                };
                self.defs
                    .tag("pattern")
                    .attr("id", &pid)
                    .attr("patternUnits", "userSpaceOnUse")
                    .attr("width", num(pw))
                    .attr("height", num(ph))
                    .attr("patternTransform", matrix(m))
                    .open();
                self.defs.tag("use").attr("href", format_args!("#{iid}")).empty();
                self.defs.close();
                Some(PaintAttrs { paint: clip_url(&pid), opacity: (p.opacity < 0.999).then(|| num(f64::from(p.opacity))), blend })
            }
            PaintKind::Pattern(pt) => {
                let (tiles, cell) = self.pattern_tiles(pt, w, h)?;
                let pid = self.id("pattern");
                self.defs
                    .tag("pattern")
                    .attr("id", &pid)
                    .attr("patternUnits", "userSpaceOnUse")
                    .attr("width", num(cell.size.0))
                    .attr("height", num(cell.size.1))
                    .attr("patternTransform", format_args!("translate({} {})", num(cell.origin.0), num(cell.origin.1)))
                    .open();
                self.defs.raw(&tiles);
                self.defs.close();
                Some(PaintAttrs { paint: clip_url(&pid), opacity: (p.opacity < 0.999).then(|| num(f64::from(p.opacity))), blend })
            }
        }
    }

    /// One cell of pattern `pt` on a `w`×`h` box: the source's vectors at each tile, or (a source
    /// with effects, which draw as page-placed images) an image of the cell.
    fn pattern_tiles(&mut self, pt: &figcraft_color::PatternPaint, w: f64, h: f64) -> Option<(String, figcraft_color::PatternCell)> {
        let doc = self.doc;
        let src = doc.node(NodeId(pt.source)).filter(|n| !n.is_page())?;
        if doc.subtree(src.id).iter().filter_map(|i| doc.node(*i)).any(|n| n.effects.iter().any(|e| e.visible)) {
            let (pix, cell, _) = figcraft_render::pattern::cell_pixmap(doc, pt, w, h, RASTER_SCALE, 0.0)?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(figcraft_render::encode_png(&pix).ok()?);
            let mut img = SvgWriter::fragment();
            img.tag("image")
                .attr("width", num(cell.size.0))
                .attr("height", num(cell.size.1))
                .attr("preserveAspectRatio", "none")
                .attr("href", format_args!("data:image/png;base64,{b64}"))
                .empty();
            return Some((img.finish(), cell));
        }
        let cell = pt.cell(src.width, src.height, w, h)?;
        let unrotated = src.transform.inverse();
        let page_body = std::mem::take(&mut self.body);
        // The source is one level below the layer it fills.
        let depth = figcraft_render::layer_depth() + 1;
        let _ = figcraft_render::pattern::nested(|| {
            for (tx, ty) in &cell.tiles {
                self.node(src.id, Affine::translate((*tx, *ty)) * Affine::scale(cell.scale) * unrotated, depth);
            }
            Some(())
        });
        Some((std::mem::replace(&mut self.body, page_body).finish(), cell))
    }

    /// The layer's filter: drop shadows (but those drawn as `exact` geometry: see
    /// [`W::drop_shadows`]) and layer blurs, as the canvas draws them: the blurs one after another,
    /// the shadows cast from and cut out by the layer they leave.
    fn filter_for(&mut self, n: &Node, exact: bool, xf: Affine) -> Option<String> {
        let fx: Vec<&figcraft_doc::Effect> = n
            .effects
            .iter()
            .filter(|e| e.visible)
            .filter(|e| !(exact && matches!(&e.kind, EffectKind::DropShadow(s) if s.spread != 0.0)))
            .filter(|e| matches!(e.kind, EffectKind::DropShadow(_) | EffectKind::LayerBlur { .. }))
            .collect();
        if fx.is_empty() {
            return None;
        }
        let fid = self.id("filter");
        // The region holds everything the layer and its effects paint (a fixed share of the
        // layer's box cuts off shadows and blurs that reach further).
        let k = xf.determinant().abs().sqrt();
        let m = figcraft_render::draw::effect_margin(n) * if k.is_finite() { k } else { 1.0 };
        let region = self
            .doc
            .world_render_bounds(n.id)
            .map(|b| figcraft_geom::transform_rect_bbox(xf * self.doc.world_transform(n.id).inverse(), b).inflate(m, m))
            .filter(|r| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()));
        let filter_tag = self.defs.tag("filter").attr("id", &fid);
        match region {
            Some(r) => filter_tag
                .attr("filterUnits", "userSpaceOnUse")
                .attr("x", num(r.x0))
                .attr("y", num(r.y0))
                .attr("width", num(r.width()))
                .attr("height", num(r.height())),
            None => filter_tag.attr("x", "-50%").attr("y", "-50%").attr("width", "200%").attr("height", "200%"),
        }
        .attr("color-interpolation-filters", "sRGB")
        .open();
        // Layer blurs one after another, each blurring what the one before it left (like the
        // renderer's layer effects); the last result is the layer drawn. A blur of 0 does nothing.
        let mut layer = "SourceGraphic".to_string();
        for (i, e) in fx.iter().enumerate() {
            if let EffectKind::LayerBlur(b) = &e.kind
                && b.sanitized().radius > 0.0
            {
                let res = format!("l{i}");
                self.defs.tag("feGaussianBlur").attr("in", &layer).attr("stdDeviation", num(b.sanitized().radius / 2.0)).attr("result", &res).empty();
                layer = res;
            }
        }
        let blurred = layer != "SourceGraphic";
        let has_shadows = fx.iter().any(|e| matches!(e.kind, EffectKind::DropShadow(_)));
        // Shadows are cast from the blurred layer's alpha, and those that don't show behind
        // transparent areas are cut out where the layer is: by what its paints cover, or by its
        // pixels (blurred).
        if blurred && has_shadows {
            self.defs
                .tag("feColorMatrix")
                .attr("in", &layer)
                .attr("type", "matrix")
                .attr("values", "0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0")
                .attr("result", "la")
                .empty();
        }
        let cover = self.cover_input(n, xf);
        let cast = if blurred { "la" } else { "SourceAlpha" };
        let knock = if cover.is_some() {
            "cover"
        } else if blurred {
            layer.as_str()
        } else {
            "SourceAlpha"
        };
        if let Some(c) = &cover {
            self.defs.raw(c);
        }
        let mut merge = Vec::new();
        for (i, e) in fx.iter().enumerate() {
            let EffectKind::DropShadow(s) = &e.kind else { continue };
            let sid = format!("s{i}");
            let src = if s.spread != 0.0 {
                let mid = format!("s{i}m");
                self.defs
                    .tag("feMorphology")
                    .attr("in", cast)
                    .attr("operator", if s.spread > 0.0 { "dilate" } else { "erode" })
                    .attr("radius", num(s.spread.abs()))
                    .attr("result", &mid)
                    .empty();
                mid
            } else {
                cast.to_string()
            };
            self.defs.tag("feOffset").attr("in", &src).attr("dx", num(s.offset.0)).attr("dy", num(s.offset.1)).empty();
            self.defs.tag("feGaussianBlur").attr("stdDeviation", num(s.radius / 2.0)).empty();
            self.defs
                .tag("feColorMatrix")
                .attr("type", "matrix")
                .attr(
                    "values",
                    format_args!(
                        "0 0 0 0 {} 0 0 0 0 {} 0 0 0 0 {} 0 0 0 {} 0",
                        num(f64::from(s.color.r)),
                        num(f64::from(s.color.g)),
                        num(f64::from(s.color.b)),
                        num(f64::from(s.color.a))
                    ),
                )
                .attr("result", &sid)
                .empty();
            if !s.show_behind {
                self.defs.tag("feComposite").attr("in", &sid).attr("in2", knock).attr("operator", "out").attr("result", &sid).empty();
            }
            merge.push(sid);
        }
        self.defs.tag("feMerge").open();
        for m in &merge {
            self.defs.tag("feMergeNode").attr("in", m).empty();
        }
        self.defs.tag("feMergeNode").attr("in", &layer).empty();
        self.defs.close();
        self.defs.close();
        Some(fid)
    }

    fn node(&mut self, id: NodeId, xf: Affine, depth: usize) {
        // What the renderer draws for this layer (images of its effects, pattern cells) and its
        // pattern sources go on from its depth: one cap for the whole export (#1082).
        figcraft_render::at_layer_depth(depth, || self.layer(id, xf, depth));
    }

    fn open_clip(&mut self, clip: Option<&str>) {
        if let Some(cid) = clip {
            self.body.tag("g").attr("clip-path", clip_url(cid)).open();
        }
    }

    fn close_clip(&mut self, clip: Option<&str>) {
        if clip.is_some() {
            self.body.close();
        }
    }

    fn layer(&mut self, id: NodeId, xf: Affine, depth: usize) {
        if depth > MAX_EXPORT_DEPTH {
            return;
        }
        let Some(n) = self.doc.node(id) else { return };
        if !n.visible || (n.is_mask && self.mask != Some(id)) {
            return;
        }
        let xf = xf * n.transform;
        let id_val = self.opts.include_ids.then(|| self.unique_id(&n.name));
        if self.raster_whole(id, n, id_val.as_deref()) {
            return;
        }
        let grains = self.grain_images(id, n);
        let blend = blend_css(n.blend);
        let style = blend
            .map(|b| format!("mix-blend-mode:{b}"))
            .or_else(|| (!grains.is_empty() || n.isolates_children()).then(|| "isolation:isolate".to_string()));
        let inherited = self.clip.clone();
        let spreads = n.effects.iter().any(|e| e.visible && matches!(&e.kind, EffectKind::DropShadow(s) if s.spread != 0.0));
        let sil = if spreads { figcraft_render::shadow::silhouette(self.doc, n) } else { None };
        if let Some(sil) = &sil {
            self.drop_shadows(n, sil, xf, inherited.as_deref());
        }
        let fid = self.filter_for(n, sil.is_some(), xf);
        let leaf = matches!(n.kind, NodeKind::BooleanOp { .. }) || n.children.is_empty();
        let isolated = n.opacity < 0.999 || style.is_some() || fid.is_some() || n.effects.iter().any(|e| e.visible);
        let (paint_clip, kid_clip) = if isolated || leaf { (None, None) } else { (inherited.clone(), inherited.clone()) };
        self.body
            .tag("g")
            .attr_opt("id", id_val.as_deref())
            .attr_opt("opacity", (n.opacity < 0.999).then(|| num(f64::from(n.opacity))))
            .attr_opt("style", style.as_deref())
            .attr_opt("filter", fid.as_deref().map(clip_url))
            .attr_opt("clip-path", (isolated || leaf).then_some(inherited.as_deref()).flatten().map(clip_url))
            .open();
        if backdrop::has_effects(n)
            && let Some(img) = self.backdrop_image(id, None)
        {
            self.body.raw(&img);
        }
        let (w, h) = (n.width, n.height);
        let outline = match &n.kind {
            NodeKind::Group | NodeKind::Page { .. } => None,
            _ => figcraft_render::node_outline(self.doc, n),
        };
        self.render_node(NodePaint { node: n, xf, depth, paint_clip, kid_clip, inherited, outline, grains, size: (w, h) });
        self.body.close();
    }

    fn render_node(&mut self, paint: NodePaint<'_>) {
        let NodePaint { node: n, xf, depth, paint_clip, kid_clip, inherited, outline, grains, size: (w, h) } = paint;
        let closed = figcraft_doc::geometry::is_closed(n) || matches!(n.kind, NodeKind::BooleanOp { .. } | NodeKind::Text(_));
        let live_text = if self.opts.outline_text { None } else { self.text_element(n, xf) };
        let run_paths = if live_text.is_none() { text_run_paths(n) } else { None };
        if let Some(t) = &live_text {
            self.body.raw(t);
        } else if let Some(runs) = &run_paths {
            // Character ranges with their own fills: one path per style, like the canvas.
            for (path, fills) in runs {
                for p in fills {
                    if let Some(a) = self.paint_attr(p, w, h) {
                        self.open_clip(paint_clip.as_deref());
                        a.apply_fill(self.body.tag("path").attr("transform", matrix(xf)).attr("d", path_d(path))).empty();
                        self.close_clip(paint_clip.as_deref());
                    }
                }
            }
        } else if let Some((path, rule)) = &outline
            && closed
        {
            if let NodeKind::Vector { network } = &n.kind
                && network.regions.iter().any(|r| r.fills.is_some() || r.paint_space.is_some())
            {
                let drawn = figcraft_doc::geometry::drawn_network(network, n.corners.smoothing);
                let net = drawn.as_ref();
                for (i, region) in net.regions.iter().enumerate() {
                    let paints = region.fills.as_deref().unwrap_or(&n.fills);
                    let fr = (region.winding == figcraft_doc::WindingRule::EvenOdd).then_some("evenodd");
                    if let Some(mut region_path) = net.region_path(i) {
                        let (pw, ph, frame) = region.paint_frame(w, h);
                        region_path.apply_affine(frame.inverse());
                        for p in paints {
                            if let Some(a) = self.paint_attr(p, pw, ph) {
                                self.open_clip(paint_clip.as_deref());
                                a.apply_fill(self.body.tag("path").attr("transform", matrix(xf * frame)).attr("d", path_d(&region_path)))
                                    .attr_opt("fill-rule", fr)
                                    .empty();
                                self.close_clip(paint_clip.as_deref());
                            }
                        }
                    }
                }
            } else {
                let fr = (*rule == figcraft_render::FillRule::EvenOdd).then_some("evenodd");
                for p in &n.fills {
                    if let Some(a) = self.paint_attr(p, w, h) {
                        self.open_clip(paint_clip.as_deref());
                        a.apply_fill(self.body.tag("path").attr("transform", matrix(xf)).attr("d", path_d(path))).attr_opt("fill-rule", fr).empty();
                        self.close_clip(paint_clip.as_deref());
                    }
                }
            }
        }
        // Inner shadows paint over the fills, under the children and strokes.
        if let Some(o) = &outline {
            self.inner_shadows(n, o, xf);
        }
        if !matches!(n.kind, NodeKind::BooleanOp { .. }) && !n.children.is_empty() {
            self.children(n, xf, depth, outline.as_ref(), kid_clip, inherited);
        }
        if n.has_visible_stroke()
            && let Some(simple) = self.opts.simplify_stroke.then(|| self.simple_stroke(n, xf)).flatten()
        {
            self.open_clip(paint_clip.as_deref());
            self.body.raw(&simple);
            self.close_clip(paint_clip.as_deref());
        } else if n.has_visible_stroke() {
            self.stroke_paths(n, xf, outline.as_ref(), closed, paint_clip.as_deref());
        }
        self.body.raw(&grains);
    }

    /// Paints the children of `n`, each mask layer wrapping the layers it covers.
    fn children(
        &mut self,
        n: &Node,
        xf: Affine,
        depth: usize,
        outline: Option<&(BezPath, figcraft_render::FillRule)>,
        mut kid_clip: Option<String>,
        inherited: Option<String>,
    ) {
        if n.clip
            && n.kind.is_frame_like()
            && let Some((p, _)) = &outline
        {
            // Its own clip, within the clips around it.
            let cid = self.id("clip");
            let mut cp = p.clone();
            cp.apply_affine(xf);
            self.defs.tag("clipPath").attr("id", &cid).attr_opt("clip-path", kid_clip.as_deref().map(clip_url)).open();
            self.defs.tag("path").attr("d", path_d(&cp)).empty();
            self.defs.close();
            kid_clip = Some(cid);
        }
        self.clip = kid_clip;
        let kids = n.children.clone();
        for run in self.doc.paint_runs(n) {
            let Some(&c) = kids.get(run.start) else { continue };
            if self.doc.node(c).is_some_and(|cn| cn.is_mask && cn.visible) {
                let kind = self.doc.node(c).map(|mn| mn.mask_type).unwrap_or_default();
                self.open_mask(c, kind, xf, depth);
                for k in kids.get(run.start + 1..run.end).unwrap_or(&[]) {
                    self.node(*k, xf, depth + 1);
                }
                self.body.close();
            } else {
                self.node(c, xf, depth + 1);
            }
        }
        self.clip = inherited;
    }

    /// The inner shadows of `n` over its `outline`.
    fn inner_shadows(&mut self, n: &Node, outline: &Outline, xf: Affine) {
        for e in n.effects.iter().filter(|e| e.visible) {
            if let EffectKind::InnerShadow(s) = &e.kind {
                self.inner_shadow(outline, s, xf);
            }
        }
    }

    /// Opens the `<g>` that the mask layer `c` clips or masks the layers after it with.
    fn open_mask(&mut self, c: NodeId, kind: figcraft_doc::MaskType, xf: Affine, depth: usize) {
        if kind == figcraft_doc::MaskType::Vector {
            // A vector mask is a clip path: its outline.
            let cid = self.id("clip");
            let outline = self.doc.node(c).and_then(|mn| Some((xf * mn.transform, figcraft_render::subtree_outline(self.doc, mn, depth + 1)?)));
            self.defs.tag("clipPath").attr("id", &cid).open();
            match &outline {
                Some((mxf, (mp, rule))) => {
                    self.defs
                        .tag("path")
                        .attr("transform", matrix(*mxf))
                        .attr("d", path_d(mp))
                        .attr_opt("clip-rule", (*rule == figcraft_render::FillRule::EvenOdd).then_some("evenodd"))
                        .empty();
                }
                None => self.defs.tag("path").attr("d", "").empty(),
            }
            self.defs.close();
            self.body.tag("g").attr("clip-path", clip_url(&cid)).open();
        } else {
            // The layer as drawn: its alpha or luminance masks.
            let mid = self.id("mask");
            let saved = std::mem::take(&mut self.body);
            let outer = self.mask.replace(c);
            let clip = self.clip.take();
            self.node(c, xf, depth + 1);
            self.clip = clip;
            self.mask = outer;
            let mask_body = std::mem::replace(&mut self.body, saved).finish();
            self.defs
                .tag("mask")
                .attr("id", &mid)
                .attr("maskUnits", "userSpaceOnUse")
                .attr_opt("style", (kind == figcraft_doc::MaskType::Alpha).then_some("mask-type:alpha"))
                .open();
            self.defs.raw(&mask_body);
            self.defs.close();
            self.body.tag("g").attr("mask", clip_url(&mid)).open();
        }
    }

    /// The visible strokes of `n` as filled outline paths.
    fn stroke_paths(&mut self, n: &Node, xf: Affine, outline: Option<&(BezPath, figcraft_render::FillRule)>, closed: bool, paint_clip: Option<&str>) {
        let geom = match &n.kind {
            NodeKind::BooleanOp { .. } | NodeKind::Text(_) => outline.map(|(p, _)| p.clone()),
            _ => figcraft_doc::geometry::stroke_geometry(n),
        };
        if let Some(g) = geom {
            let area = figcraft_render::draw::stroke_area(n, &g, closed);
            let even = n.stroke.sides.is_some() || (n.stroke.dashes.is_empty() && closed && n.stroke.align != figcraft_doc::StrokeAlign::Center);
            let fr = even.then_some("evenodd");
            for p in &n.strokes {
                if let Some(a) = self.paint_attr(p, n.width, n.height) {
                    self.open_clip(paint_clip);
                    a.apply_fill(self.body.tag("path").attr("transform", matrix(xf)).attr("d", path_d(&area))).attr_opt("fill-rule", fr).empty();
                    self.close_clip(paint_clip);
                }
            }
        }
    }

    /// `name` as an `id` not used yet in this file (spaces as underscores; repeats get `_2`…).
    fn unique_id(&mut self, name: &str) -> String {
        // As written: what `esc` leaves out isn't part of it.
        let base: String =
            name.chars().take(200).map(|c| if c.is_whitespace() || c.is_control() { '_' } else { c }).filter(|&c| xml_char(c)).collect();
        let base = if base.is_empty() { "layer".to_string() } else { base };
        let next = self.next_suffix.entry(base.clone()).or_insert(1);
        let id = loop {
            let k = *next;
            *next += 1;
            let candidate = if k == 1 { base.clone() } else { format!("{base}_{k}") };
            if !self.used_ids.contains(&candidate) {
                break candidate;
            }
        };
        self.used_ids.insert(id.clone());
        id
    }

    /// Text layer `n` as a `<text>` element, one `<tspan>` per laid-out line, in its first fill.
    /// shortcut: one style for the whole layer (the first run's family, size and weight) and text
    /// on a path stays outlined; write a `<tspan>` per styled run when mixed styles are needed.
    fn text_element(&mut self, n: &Node, xf: Affine) -> Option<String> {
        let NodeKind::Text(t) = &n.kind else { return None };
        if t.path.is_some() {
            return None;
        }
        let st = &t.style;
        let fill = n.fills.iter().find_map(|p| self.paint_attr(p, n.width, n.height));
        let chars: Vec<char> = t.characters.chars().collect();
        let mut spans = SvgWriter::fragment();
        let mut gxf = xf;
        if let Some((layout, dy)) = figcraft_text::layout_node(n) {
            gxf = xf * Affine::translate((0.0, dy));
            for line in &layout.lines {
                let glyphs = layout.glyphs.get(line.glyph_start..line.glyph_end).unwrap_or(&[]);
                let run_fills = !line.rtl
                    && !glyphs.iter().any(|g| g.rtl)
                    && glyphs.iter().any(|g| layout.styles.get(g.style).is_some_and(|s| s.fills.is_some()));
                if run_fills {
                    // One tspan per run of glyphs in the same style, each in its own fill.
                    let mut i = 0;
                    let mut start = line.char_start;
                    while i < glyphs.len() {
                        let style = glyphs[i].style;
                        let mut j = i + 1;
                        while j < glyphs.len() && glyphs[j].style == style {
                            j += 1;
                        }
                        let end = glyphs.get(j).map_or(line.char_end, |g| g.char_start).max(start).min(chars.len());
                        let text: String = chars.get(start.min(end)..end).unwrap_or(&[]).iter().collect();
                        let prev = start.checked_sub(1).and_then(|k| chars.get(k)).copied();
                        let before = start.checked_sub(2).and_then(|k| chars.get(k)).copied();
                        let text = st.case.apply_between(text.trim_end_matches(['\n', '\r']), before, prev);
                        let own = layout
                            .styles
                            .get(style)
                            .and_then(|s| s.fills.as_ref())
                            .map(|fills| fills.iter().find_map(|p| self.paint_attr(p, n.width, n.height)));
                        let tspan = spans.tag("tspan").attr("x", num(glyphs[i].x)).attr("y", num(glyphs[i].y));
                        let tspan = match own {
                            Some(Some(a)) => a.apply_fill(tspan),
                            Some(None) => tspan.attr("fill", "none"),
                            None => tspan,
                        };
                        tspan.text(&text);
                        start = end;
                        i = j;
                    }
                    continue;
                }
                let text: String = chars.get(line.char_start..line.char_end.min(chars.len())).unwrap_or(&[]).iter().collect();
                let prev = line.char_start.checked_sub(1).and_then(|k| chars.get(k)).copied();
                let before = line.char_start.checked_sub(2).and_then(|k| chars.get(k)).copied();
                let text = st.case.apply_between(text.trim_end_matches(['\n', '\r']), before, prev);
                let (x, y) = layout.glyphs.get(line.glyph_start).map_or((0.0, line.baseline), |g| (g.x, g.y));
                spans.tag("tspan").attr("x", num(x)).attr("y", num(y)).text(&text);
            }
        }
        if spans.is_empty() {
            // No fonts laid it out: one line on the first baseline.
            spans.tag("tspan").attr("x", "0").attr("y", num(st.font_size * 0.8)).text(&st.case.apply(&t.characters));
        }
        let spacing = match st.letter_spacing {
            figcraft_doc::LetterSpacing::Pixels(v) if v != 0.0 => Some(num(v)),
            figcraft_doc::LetterSpacing::Percent(p) if p != 0.0 => Some(num(p / 100.0 * st.font_size)),
            _ => None,
        };
        let mut out = SvgWriter::fragment();
        let text_tag = out
            .tag("text")
            .attr("transform", matrix(gxf))
            .attr("font-family", &st.font_family)
            .attr("font-size", num(st.font_size))
            .attr("font-weight", st.weight())
            .attr_opt("font-style", st.italic().then_some("italic"))
            .attr_opt("letter-spacing", spacing.as_deref());
        let text_tag = match &fill {
            Some(a) => a.apply_fill(text_tag),
            None => text_tag.attr("fill", "none"),
        };
        text_tag.attr("xml:space", "preserve").open();
        out.raw(&spans.finish());
        out.close();
        Some(out.finish())
    }

    /// `n`'s strokes as `stroke` attributes on a path, when that draws exactly what the outline
    /// does: a uniform weight (no per-side weights, width profile, brush or arrowheads), centred on
    /// the path, or inside or outside a box or ellipse (drawn centred on it grown or shrunk by half
    /// the weight). `None` otherwise.
    fn simple_stroke(&mut self, n: &Node, xf: Affine) -> Option<String> {
        use figcraft_doc::{StrokeAlign, StrokeCap, StrokeJoin};
        let k = &n.stroke;
        let w = k.weight;
        if k.sides.is_some()
            || k.profile().is_some()
            || k.brush.is_some()
            || k.cap_start.is_arrow()
            || k.cap_end.is_arrow()
            || matches!(&n.kind, NodeKind::Vector { network } if network.has_point_styles(k.join))
            || !(w.is_finite() && w > 0.0)
        {
            return None;
        }
        let path = match k.align {
            StrokeAlign::Center => match n.kind {
                NodeKind::Text(_) | NodeKind::BooleanOp { .. } | NodeKind::Group => return None,
                _ => figcraft_doc::geometry::stroke_geometry(n)?,
            },
            align if k.dashes.is_empty() => {
                let d = if align == StrokeAlign::Inside { -w / 2.0 } else { w / 2.0 };
                let r = n.local_rect().inflate(d, d);
                if r.width() <= 0.0 || r.height() <= 0.0 {
                    return None;
                }
                match n.kind {
                    NodeKind::Ellipse { sweep, inner, .. } if inner == 0.0 && figcraft_geom::full_sweep(sweep) => {
                        figcraft_geom::kurbo::Shape::to_path(&figcraft_geom::kurbo::Ellipse::from_rect(r), 0.1)
                    }
                    NodeKind::Rectangle | NodeKind::Frame | NodeKind::Component { .. } | NodeKind::Instance { .. } if n.corners.smoothing == 0.0 => {
                        // A rounded corner keeps a radius only while the shifted radius stays positive.
                        let radii = n.corners.radii.map(|c| if c > 0.0 { c + d } else { 0.0 });
                        if n.corners.radii.iter().zip(radii).any(|(c, r)| *c > 0.0 && r <= 0.0) {
                            return None;
                        }
                        let [tl, tr, br, bl] = radii;
                        figcraft_geom::kurbo::Shape::to_path(&figcraft_geom::kurbo::RoundedRect::from_rect(r, (tl, tr, br, bl)), 0.1)
                    }
                    _ => return None,
                }
            }
            _ => return None,
        };
        let cap = match k.band_caps()[0] {
            StrokeCap::Round => Some("round"),
            StrokeCap::Square => Some("square"),
            _ => None,
        };
        let miter = (k.join == StrokeJoin::Miter && (k.miter_limit - 4.0).abs() > 1e-9).then(|| num(k.miter_limit.max(1.0)));
        let join = match k.join {
            StrokeJoin::Miter => None,
            StrokeJoin::Round => Some("round"),
            StrokeJoin::Bevel => Some("bevel"),
        };
        let pieces = dashed_pieces(path, k);
        let mut out = SvgWriter::fragment();
        for p in &n.strokes {
            if let Some(a) = self.paint_attr(p, n.width, n.height) {
                for piece in &pieces {
                    a.apply_stroke(out.tag("path").attr("transform", matrix(xf)).attr("d", &piece.d).attr("fill", "none"))
                        .attr("stroke-width", num(w))
                        .attr_opt("stroke-linecap", cap)
                        .attr_opt("stroke-linejoin", join)
                        .attr_opt("stroke-miterlimit", miter.as_deref())
                        .attr_opt("stroke-dasharray", piece.dasharray.as_deref())
                        .attr_opt("stroke-dashoffset", piece.dashoffset.as_deref())
                        .empty();
                }
            }
        }
        Some(out.finish())
    }

    /// `<image>` markup of `pix`, a render of `view`'s page area, with optional `opacity` and
    /// `style`; `None` when it is empty.
    fn image(&self, pix: &Pixmap, view: &View, opacity: Option<f32>, style: Option<&str>) -> Option<String> {
        if pix.data().iter().skip(3).step_by(4).all(|a| *a == 0) {
            return None;
        }
        let png = figcraft_render::encode_png(pix).ok()?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);
        let t = self.page * Affine::translate(view.origin) * Affine::scale(1.0 / view.scale.max(1e-9));
        let mut out = SvgWriter::fragment();
        out.tag("image")
            .attr("transform", matrix(t))
            .attr("width", pix.width())
            .attr("height", pix.height())
            .attr_opt("opacity", opacity.filter(|o| *o < 0.999).map(|o| num(f64::from(o))))
            .attr_opt("style", style)
            .attr("href", format_args!("data:image/png;base64,{b64}"))
            .empty();
        Some(out.finish())
    }

    /// Node `id`'s background blurs and glass as an image over its shape, drawn by the renderer
    /// from what lies under it among the exported layers (`opacity`: the layer's, when the image
    /// isn't inside its group).
    fn backdrop_image(&self, id: NodeId, opacity: Option<f32>) -> Option<String> {
        let view = raster_view(backdrop::shape_bounds(self.doc, id)?, RASTER_SCALE)?;
        let pix = backdrop::effects_beneath(self.doc, self.roots, id, &view, None)?;
        self.image(&pix, &view, opacity, None)
    }

    /// Layers whose look SVG can't build (texture scatters their pixels, a progressive blur varies
    /// across them, noise over a blur depends on their order) are drawn by the renderer and
    /// embedded whole, over their backdrop effects. False when the layer isn't one of them (or
    /// can't be rendered): it is written as vectors.
    fn raster_whole(&mut self, id: NodeId, n: &Node, id_val: Option<&str>) -> bool {
        let has = |f: &dyn Fn(&EffectKind) -> bool| n.effects.iter().any(|e| e.visible && f(&e.kind));
        let whole = has(&|k| matches!(k, EffectKind::Texture(_)))
            || has(&|k| matches!(k, EffectKind::LayerBlur(b) if b.is_progressive()))
            || (has(&|k| matches!(k, EffectKind::Noise(_))) && has(&|k| matches!(k, EffectKind::LayerBlur(_))))
            // A group's inner shadows are cast by its layers' pixels, over them; an image fill's by
            // its alpha.
            || (has(&|k| matches!(k, EffectKind::InnerShadow(_))) && figcraft_render::shadow::inner_from_pixels(self.doc, n));
        if !whole {
            return false;
        }
        let Some(view) = figcraft_render::raster_bounds(self.doc, id).and_then(|b| raster_view(b, RASTER_SCALE)) else { return false };
        // The renderer applies the layer's opacity; its blend mode applies to the SVG beneath.
        let Some(img) = figcraft_render::render_alone(self.doc, id, &view, Alone::WithEffects).and_then(|p| self.image(&p, &view, None, None)) else {
            return false;
        };
        let back = if backdrop::has_effects(n) { self.backdrop_image(id, Some(n.opacity.clamp(0.0, 1.0))) } else { None };
        let style = blend_css(n.blend).map(|b| format!("mix-blend-mode:{b}"));
        let clip = self.clip.as_deref().map(clip_url);
        self.body.tag("g").attr_opt("id", id_val).attr_opt("style", style.as_deref()).attr_opt("clip-path", clip.as_deref()).open();
        if let Some(b) = &back {
            self.body.raw(b);
        }
        self.body.raw(&img);
        self.body.close();
        true
    }

    /// Node `id`'s noise grains as images over the layer (crisp pixels, the noise's blend mode),
    /// drawn by the renderer from the layer's content (empty without noise).
    fn grain_images(&self, id: NodeId, n: &Node) -> String {
        let noises: Vec<&figcraft_doc::Noise> = n
            .effects
            .iter()
            .filter(|e| e.visible)
            .filter_map(|e| match &e.kind {
                EffectKind::Noise(nz) => Some(nz),
                _ => None,
            })
            .collect();
        if noises.is_empty() {
            return String::new();
        }
        // At least two pixels a grain (grains are twice the noise size across), up to 4 per unit.
        let world = self.doc.world_transform(id);
        let sc = world.determinant().abs().sqrt();
        let cell = noises.iter().map(|nz| nz.grain().0.min(nz.grain().1) * 2.0).fold(f64::INFINITY, f64::min) * sc;
        let k = if cell.is_finite() && cell > 0.0 { (2.0 / cell).clamp(RASTER_SCALE, 4.0) } else { RASTER_SCALE };
        let Some(view) = self.doc.world_render_bounds(id).and_then(|b| raster_view(b, k)) else { return String::new() };
        let Some(layer) = figcraft_render::render_alone(self.doc, id, &view, Alone::Plain) else { return String::new() };
        let xf = view.transform() * world;
        let mut out = String::new();
        for nz in noises {
            let style = match blend_css(nz.blend) {
                Some(b) => format!("image-rendering:pixelated;mix-blend-mode:{b}"),
                None => "image-rendering:pixelated".to_string(),
            };
            if let Some(img) = figcraft_render::grain::grains(&layer, nz, xf, id.0 as u32).and_then(|g| self.image(&g, &view, None, Some(&style))) {
                out.push_str(&img);
            }
        }
        out
    }

    /// Markup painting `outlines` (local, through `xf`) grown (`spread > 0`) or shrunk by `spread`
    /// local units in opaque `color`: the outline filled and stroked twice the spread wide with
    /// mitred joins (shrinking: that band masked away), like the renderer's spread.
    fn spread_paths(&mut self, outlines: &[Outline], spread: f64, xf: Affine, color: &str) -> String {
        let paths = |fill: &str, stroke: Option<(&str, f64)>| -> String {
            let mut s = SvgWriter::fragment();
            for (p, rule) in outlines {
                s.tag("path")
                    .attr("transform", matrix(xf))
                    .attr("d", path_d(p))
                    .attr("fill", fill)
                    .attr_opt("fill-rule", (*rule == figcraft_render::FillRule::EvenOdd).then_some("evenodd"))
                    .empty();
            }
            if let Some((c, half)) = stroke {
                for (p, _) in outlines {
                    s.tag("path")
                        .attr("transform", matrix(xf))
                        .attr("d", path_d(p))
                        .attr("fill", "none")
                        .attr("stroke", c)
                        .attr("stroke-width", num(half * 2.0))
                        .attr("stroke-linejoin", "miter")
                        .attr("stroke-miterlimit", num(SPREAD_MITER_LIMIT))
                        .empty();
                }
            }
            s.finish()
        };
        let half = if spread.is_finite() { spread.abs() } else { 0.0 };
        if spread >= 0.0 || half <= 1e-6 {
            return paths(color, (half > 1e-6).then_some((color, half)));
        }
        let bounds =
            outlines.iter().map(|(p, _)| figcraft_geom::transform_rect_bbox(xf, figcraft_geom::Shape::bounding_box(p))).reduce(|a, b| a.union(b));
        let Some(b) = bounds.map(|b| b.inflate(1.0, 1.0)) else { return String::new() };
        let mid = self.id("mask");
        self.defs
            .tag("mask")
            .attr("id", &mid)
            .attr("maskUnits", "userSpaceOnUse")
            .attr("x", num(b.x0))
            .attr("y", num(b.y0))
            .attr("width", num(b.width()))
            .attr("height", num(b.height()))
            .open();
        self.defs.raw(&paths("white", Some(("black", half))));
        self.defs.close();
        let mut out = SvgWriter::fragment();
        out.tag("g").attr("mask", clip_url(&mid)).open();
        out.raw(&paths(color, None));
        out.close();
        out.finish()
    }

    /// A Gaussian blur filter over `region` (page units), cut out by `cover` (a filter input
    /// named `cover`, see [`W::cover_input`]); `None` for no blur and no cover.
    fn blur_filter(&mut self, radius: f64, region: Rect, cover: Option<&str>) -> Option<String> {
        let blur = radius.is_finite() && radius > 0.0;
        if !blur && cover.is_none() {
            return None;
        }
        let fid = self.id("filter");
        self.defs
            .tag("filter")
            .attr("id", &fid)
            .attr("filterUnits", "userSpaceOnUse")
            .attr("x", num(region.x0))
            .attr("y", num(region.y0))
            .attr("width", num(region.width()))
            .attr("height", num(region.height()))
            .attr("color-interpolation-filters", "sRGB")
            .open();
        if blur {
            self.defs.tag("feGaussianBlur").attr("stdDeviation", num(radius / 2.0)).attr("result", "b").empty();
        }
        if let Some(c) = cover {
            self.defs.raw(c);
            self.defs.tag("feComposite").attr("in", if blur { "b" } else { "SourceGraphic" }).attr("in2", "cover").attr("operator", "out").empty();
        }
        self.defs.close();
        Some(fid)
    }

    /// The area `n`'s paints cover, as a filter input named `cover` (an image of its outlines
    /// through `xf`), when its drop shadows hide behind it there (see
    /// [`shadow::hiding_cover`](figcraft_render::shadow::hiding_cover)).
    fn cover_input(&self, n: &Node, xf: Affine) -> Option<String> {
        let cover = figcraft_render::shadow::hiding_cover(self.doc, n)?;
        let b = cover
            .iter()
            .map(|(p, _)| figcraft_geom::transform_rect_bbox(xf, figcraft_geom::Shape::bounding_box(p)))
            .reduce(|a, b| a.union(b))?
            .inflate(1.0, 1.0);
        if !(b.width() > 0.0 && b.height() > 0.0 && [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite())) {
            return None;
        }
        let mut img = SvgWriter::fragment();
        img.tag("svg")
            .attr("xmlns", SVG_NS)
            .attr("viewBox", format_args!("{} {} {} {}", num(b.x0), num(b.y0), num(b.width()), num(b.height())))
            .attr("width", num(b.width()))
            .attr("height", num(b.height()))
            .open();
        for (p, rule) in &cover {
            img.tag("path")
                .attr("transform", matrix(xf))
                .attr("d", path_d(p))
                .attr_opt("fill-rule", (*rule == figcraft_render::FillRule::EvenOdd).then_some("evenodd"))
                .empty();
        }
        img.close();
        let b64 = base64::engine::general_purpose::STANDARD.encode(img.finish());
        let mut out = SvgWriter::fragment();
        out.tag("feImage")
            .attr("href", format_args!("data:image/svg+xml;base64,{b64}"))
            .attr("x", num(b.x0))
            .attr("y", num(b.y0))
            .attr("width", num(b.width()))
            .attr("height", num(b.height()))
            .attr("preserveAspectRatio", "none")
            .attr("result", "cover")
            .empty();
        Some(out.finish())
    }

    /// Drop shadows with spread, as geometry under the layer: its silhouette offset by the
    /// spread (exactly, like the renderer), moved, blurred and faded.
    fn drop_shadows(&mut self, n: &Node, sil: &[Outline], xf: Affine, clip: Option<&str>) {
        for e in n.effects.iter().filter(|e| e.visible) {
            let EffectKind::DropShadow(s) = &e.kind else { continue };
            if s.spread == 0.0 || !(s.offset.0.is_finite() && s.offset.1.is_finite()) {
                continue;
            }
            let at = Affine::translate(s.offset) * xf;
            let Some(b) =
                sil.iter().map(|(p, _)| figcraft_geom::transform_rect_bbox(at, figcraft_geom::Shape::bounding_box(p))).reduce(|a, b| a.union(b))
            else {
                continue;
            };
            // The spread grows the outline in its own units (mitres up to the limit), drawn through
            // `xf`: as far across and down as a local step that long goes. The blur is in page units.
            let spread = if s.spread.is_finite() { s.spread.abs() * SPREAD_MITER_LIMIT } else { 0.0 };
            let [a, b_, c, d, _, _] = xf.as_coeffs();
            let blur = s.radius.abs() * 1.5 + 2.0;
            let region = b.inflate(spread * a.hypot(c) + blur, spread * b_.hypot(d) + blur);
            let cover = if s.show_behind { None } else { self.cover_input(n, xf) };
            let filter = self.blur_filter(s.radius, region, cover.as_deref());
            let op = f64::from(s.color.a * n.opacity.clamp(0.0, 1.0));
            let style = blend_css(s.blend).map(|b| format!("mix-blend-mode:{b}"));
            let body = self.spread_paths(sil, s.spread, at, &hex(s.color));
            self.body.tag("g").attr("opacity", num(op)).attr_opt("style", style.as_deref()).attr_opt("clip-path", clip.map(clip_url)).open();
            self.body.tag("g").attr_opt("filter", filter.as_deref().map(clip_url)).open();
            self.body.raw(&body);
            self.body.close();
            self.body.close();
        }
    }

    /// An inner shadow over the layer: everything outside its outline shrunk by the spread and
    /// moved, blurred, tinted and clipped to the outline (the renderer's inner shadow).
    fn inner_shadow(&mut self, o: &Outline, s: &Shadow, xf: Affine) {
        if !(s.offset.0.is_finite() && s.offset.1.is_finite()) {
            return;
        }
        let b = figcraft_geom::transform_rect_bbox(xf, figcraft_geom::Shape::bounding_box(&o.0));
        let reach = s.spread.abs() * SPREAD_MITER_LIMIT + s.radius.abs() * 1.5 + s.offset.0.abs().max(s.offset.1.abs()) + 2.0;
        let r = b.inflate(reach, reach);
        let cr = (o.1 == figcraft_render::FillRule::EvenOdd).then_some("evenodd");
        let cid = self.id("clip");
        self.defs.tag("clipPath").attr("id", &cid).open();
        self.defs.tag("path").attr("transform", matrix(xf)).attr("d", path_d(&o.0)).attr_opt("clip-rule", cr).empty();
        self.defs.close();
        let hole = self.spread_paths(std::slice::from_ref(o), -s.spread, Affine::translate(s.offset) * xf, "black");
        let mid = self.id("mask");
        let rect = |w: &mut SvgWriter, fill: &str| {
            w.tag("rect")
                .attr("x", num(r.x0))
                .attr("y", num(r.y0))
                .attr("width", num(r.width()))
                .attr("height", num(r.height()))
                .attr("fill", fill)
                .empty();
        };
        self.defs
            .tag("mask")
            .attr("id", &mid)
            .attr("maskUnits", "userSpaceOnUse")
            .attr("x", num(r.x0))
            .attr("y", num(r.y0))
            .attr("width", num(r.width()))
            .attr("height", num(r.height()))
            .open();
        rect(&mut self.defs, "white");
        self.defs.raw(&hole);
        self.defs.close();
        let filter = self.blur_filter(s.radius, r, None);
        self.body.tag("g").attr("clip-path", clip_url(&cid)).open();
        self.body.tag("g").attr("opacity", num(f64::from(s.color.a))).attr_opt("filter", filter.as_deref().map(clip_url)).open();
        self.body.tag("g").attr("mask", clip_url(&mid)).open();
        rect(&mut self.body, &hex(s.color));
        self.body.close();
        self.body.close();
        self.body.close();
    }
}

fn sniff_mime(b: &[u8]) -> &'static str {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if b.starts_with(&[0xFF, 0xD8]) {
        "image/jpeg"
    } else if b.starts_with(b"GIF") {
        "image/gif"
    } else if b.len() > 12 && b.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        "application/octet-stream"
    }
}

/// Export `ids` (in z-order) as one SVG document covering their export bounds.
/// Returns `None` for an empty or over-depth export. Use [`try_export_svg`] for the reason.
pub fn export_svg(doc: &Document, ids: &[NodeId], opts: &SvgOptions) -> Option<String> {
    try_export_svg(doc, ids, opts).ok()
}

/// Like [`export_svg`], with an actionable error when the visible subtree is too deep. No partial
/// SVG is returned; the depth limit is relative to each exported root, not the document's page.
pub fn try_export_svg(doc: &Document, ids: &[NodeId], opts: &SvgOptions) -> Result<String, SvgError> {
    check_export_depth(doc, ids)?;
    let bounds: Rect = match opts.bounds {
        Some(b) if [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) => b,
        _ => ids.iter().filter_map(|i| figcraft_render::export_bounds(doc, *i)).reduce(|a, b| a.union(b)).ok_or(SvgError::Empty)?,
    };
    let origin = Affine::translate((-bounds.x0, -bounds.y0));
    let mut w = W {
        doc,
        roots: ids,
        page: origin,
        defs: SvgWriter::fragment(),
        body: SvgWriter::fragment(),
        next: 0,
        opts,
        mask: None,
        clip: None,
        used_ids: Default::default(),
        next_suffix: Default::default(),
        images: Default::default(),
    };
    for id in ids {
        let parent_xf = doc.node(*id).and_then(|n| n.parent).map(|p| doc.world_transform(p)).unwrap_or(Affine::IDENTITY);
        w.node(*id, origin * parent_xf, 0);
    }
    let (bw, bh) = (bounds.width().max(0.0), bounds.height().max(0.0));
    let mut out = SvgWriter::fragment();
    out.tag("svg")
        .attr("width", num(bw))
        .attr("height", num(bh))
        .attr("viewBox", format_args!("0 0 {} {}", num(bw), num(bh)))
        .attr("fill", "none")
        .attr("xmlns", SVG_NS)
        .open();
    out.raw(&w.body.finish());
    let defs = w.defs.finish();
    if !defs.is_empty() {
        out.tag("defs").open();
        out.raw(&defs);
        out.close();
    }
    out.close();
    let mut svg = out.finish();
    svg.push('\n');
    Ok(svg)
}

/// Text layer `n`'s glyph outlines grouped by style, each with the fills it paints in (its own, else
/// the layer's), in layer space. `None` unless some character range has fills of its own.
fn text_run_paths(n: &Node) -> Option<Vec<(BezPath, Vec<Paint>)>> {
    let NodeKind::Text(_) = &n.kind else { return None };
    let (layout, dy) = figcraft_text::layout_node(n)?;
    if !layout.styles.iter().any(|s| s.fills.is_some()) {
        return None;
    }
    let mut by_style: std::collections::BTreeMap<usize, BezPath> = std::collections::BTreeMap::new();
    for g in &layout.glyphs {
        by_style.entry(g.style).or_default().extend(g.path().elements().iter().copied());
    }
    for d in &layout.decorations {
        by_style.entry(d.style).or_default().extend(d.path().elements().iter().copied());
    }
    Some(
        by_style
            .into_iter()
            .map(|(k, mut path)| {
                path.apply_affine(Affine::translate((0.0, dy)));
                let fills = layout.styles.get(k).and_then(|s| s.fills.clone()).unwrap_or_else(|| n.fills.clone());
                (path, fills)
            })
            .collect(),
    )
}

#[cfg(test)]
mod depth_tests;
#[cfg(test)]
mod effects_tests;
#[cfg(test)]
mod group_effects_tests;
#[cfg(test)]
mod image_shadow_tests;
#[cfg(test)]
mod mask_export_tests;
#[cfg(test)]
mod pattern_depth_tests;
#[cfg(test)]
mod raster_fx_tests;
#[cfg(test)]
mod tests;
