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

use std::fmt::Write as _;

use base64::Engine as _;
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
    paint_clip: String,
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
    defs: String,
    body: String,
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
}

/// ` clip-path="url(#…)"` for a clip path id (else nothing).
fn clip_attr(clip: &Option<String>) -> String {
    clip.as_ref().map(|c| format!(" clip-path=\"url(#{c})\"")).unwrap_or_default()
}

/// `el` in a group with `clip` (a ` clip-path` attribute, or nothing). A clip path is in page
/// coordinates, and on an element with a `transform` it would be read after that transform.
fn clipped(clip: &str, el: String) -> String {
    if clip.is_empty() { el } else { format!("<g{clip}>{el}</g>") }
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
    use figcraft_geom::kurbo::PathEl::*;
    let mut s = String::new();
    for el in p.elements() {
        match *el {
            MoveTo(a) => {
                let _ = write!(s, "M{} {}", num(a.x), num(a.y));
            }
            LineTo(a) => {
                let _ = write!(s, "L{} {}", num(a.x), num(a.y));
            }
            QuadTo(a, b) => {
                let _ = write!(s, "Q{} {} {} {}", num(a.x), num(a.y), num(b.x), num(b.y));
            }
            CurveTo(a, b, c) => {
                let _ = write!(s, "C{} {} {} {} {} {}", num(a.x), num(a.y), num(b.x), num(b.y), num(c.x), num(c.y));
            }
            ClosePath => s.push('Z'),
        }
    }
    s
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

/// ` style="mix-blend-mode:…"` for a blend mode other than normal (else nothing).
fn blend_style(b: figcraft_color::BlendMode) -> String {
    if matches!(b, figcraft_color::BlendMode::Normal | figcraft_color::BlendMode::PassThrough) {
        String::new()
    } else {
        format!(" style=\"mix-blend-mode:{}\"", b.css())
    }
}

/// `s` escaped for XML text and attribute values; characters XML 1.0 forbids (control characters
/// other than tab, newline and carriage return, U+FFFE and U+FFFF) are left out.
fn esc(s: &str) -> String {
    s.chars().filter(|&c| xml_char(c)).collect::<String>().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Whether XML 1.0 allows `c` (see [`esc`]).
fn xml_char(c: char) -> bool {
    !matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}')
}

fn hex(c: Color) -> String {
    format!("#{}", c.to_hex())
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

    /// `fill="..." [fill-opacity]` for a paint on a w×h box; `None` when it paints nothing.
    fn paint_attr(&mut self, p: &Paint, w: f64, h: f64) -> Option<String> {
        if !p.visible || p.opacity <= 0.0 {
            return None;
        }
        let blend = blend_style(p.blend);
        match &p.kind {
            PaintKind::Solid { color } => {
                let a = color.a * p.opacity;
                let op = if a < 0.999 { format!(" fill-opacity=\"{}\"", num(f64::from(a))) } else { String::new() };
                Some(format!("fill=\"{}\"{op}{blend}", hex(*color)))
            }
            PaintKind::Gradient(g) => {
                let gid = self.id("paint");
                let stops: String = g
                    .sorted_stops()
                    .iter()
                    .map(|s| {
                        let a = s.color.a * p.opacity;
                        let op = if a < 0.999 { format!(" stop-opacity=\"{}\"", num(f64::from(a))) } else { String::new() };
                        format!("<stop offset=\"{}\" stop-color=\"{}\"{op}/>", num(f64::from(s.position)), hex(s.color))
                    })
                    .collect();
                let (f, t, k) = ((g.from.0 * w, g.from.1 * h), (g.to.0 * w, g.to.1 * h), (g.width.0 * w, g.width.1 * h));
                match g.kind {
                    GradientKind::Linear => {
                        let _ = write!(
                            self.defs,
                            "<linearGradient id=\"{gid}\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" gradientUnits=\"userSpaceOnUse\">{stops}</linearGradient>",
                            num(f.0),
                            num(f.1),
                            num(t.0),
                            num(t.1)
                        );
                    }
                    _ => {
                        // Unit circle mapped onto the handle ellipse.
                        let m = Affine::new([t.0 - f.0, t.1 - f.1, k.0 - f.0, k.1 - f.1, f.0, f.1]);
                        let _ = write!(
                            self.defs,
                            "<radialGradient id=\"{gid}\" cx=\"0\" cy=\"0\" r=\"1\" gradientUnits=\"userSpaceOnUse\" gradientTransform=\"{}\">{stops}</radialGradient>",
                            matrix(m)
                        );
                    }
                }
                Some(format!("fill=\"url(#{gid})\"{blend}"))
            }
            PaintKind::Image(img) => {
                let data = self.doc.images.get(&img.image)?;
                let pid = self.id("pattern");
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
                let t = figcraft_render::paint::image_transform(img, f64::from(data.width), f64::from(data.height), w, h);
                let m = Affine::new([f64::from(t.sx), f64::from(t.ky), f64::from(t.kx), f64::from(t.sy), f64::from(t.tx), f64::from(t.ty)]);
                let (pw, ph) = if img.scale_mode == figcraft_color::ImageScaleMode::Tile {
                    (f64::from(data.width), f64::from(data.height))
                } else {
                    (f64::from(data.width) * 4.0, f64::from(data.height) * 4.0)
                };
                let _ = write!(
                    self.defs,
                    "<pattern id=\"{pid}\" patternUnits=\"userSpaceOnUse\" width=\"{}\" height=\"{}\" patternTransform=\"{}\"><image id=\"{iid}\" width=\"{}\" height=\"{}\" href=\"data:{mime};base64,{b64}\"/></pattern>",
                    num(pw),
                    num(ph),
                    matrix(m),
                    data.width,
                    data.height
                );
                let op = if p.opacity < 0.999 { format!(" fill-opacity=\"{}\"", num(f64::from(p.opacity))) } else { String::new() };
                Some(format!("fill=\"url(#{pid})\"{op}{blend}"))
            }
            PaintKind::Pattern(pt) => {
                let (tiles, cell) = self.pattern_tiles(pt, w, h)?;
                let pid = self.id("pattern");
                let _ = write!(
                    self.defs,
                    "<pattern id=\"{pid}\" patternUnits=\"userSpaceOnUse\" width=\"{}\" height=\"{}\" patternTransform=\"translate({} {})\">{tiles}</pattern>",
                    num(cell.size.0),
                    num(cell.size.1),
                    num(cell.origin.0),
                    num(cell.origin.1)
                );
                let op = if p.opacity < 0.999 { format!(" fill-opacity=\"{}\"", num(f64::from(p.opacity))) } else { String::new() };
                Some(format!("fill=\"url(#{pid})\"{op}{blend}"))
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
            let img = format!(
                "<image width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" href=\"data:image/png;base64,{b64}\"/>",
                num(cell.size.0),
                num(cell.size.1)
            );
            return Some((img, cell));
        }
        let cell = pt.cell(src.width, src.height, w, h)?;
        let unrotated = src.transform.inverse();
        let page_body = std::mem::take(&mut self.body);
        let _ = figcraft_render::pattern::nested(|| {
            for (tx, ty) in &cell.tiles {
                self.node(src.id, Affine::translate((*tx, *ty)) * Affine::scale(cell.scale) * unrotated, 0);
            }
            Some(())
        });
        Some((std::mem::replace(&mut self.body, page_body), cell))
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
        let mut f = match region {
            Some(r) => format!(
                "<filter id=\"{fid}\" filterUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" color-interpolation-filters=\"sRGB\">",
                num(r.x0),
                num(r.y0),
                num(r.width()),
                num(r.height())
            ),
            None => format!("<filter id=\"{fid}\" x=\"-50%\" y=\"-50%\" width=\"200%\" height=\"200%\" color-interpolation-filters=\"sRGB\">"),
        };
        // Layer blurs one after another, each blurring what the one before it left (like the
        // renderer's layer effects); the last result is the layer drawn. A blur of 0 does nothing.
        let mut layer = "SourceGraphic".to_string();
        for (i, e) in fx.iter().enumerate() {
            if let EffectKind::LayerBlur(b) = &e.kind
                && b.sanitized().radius > 0.0
            {
                let _ = write!(f, "<feGaussianBlur in=\"{layer}\" stdDeviation=\"{}\" result=\"l{i}\"/>", num(b.sanitized().radius / 2.0));
                layer = format!("l{i}");
            }
        }
        let blurred = layer != "SourceGraphic";
        let has_shadows = fx.iter().any(|e| matches!(e.kind, EffectKind::DropShadow(_)));
        // Shadows are cast from the blurred layer's alpha, and those that don't show behind
        // transparent areas are cut out where the layer is: by what its paints cover, or by its
        // pixels (blurred).
        if blurred && has_shadows {
            let _ = write!(f, "<feColorMatrix in=\"{layer}\" type=\"matrix\" values=\"0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0\" result=\"la\"/>");
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
        f.push_str(cover.as_deref().unwrap_or_default());
        let mut merge = Vec::new();
        for (i, e) in fx.iter().enumerate() {
            if let EffectKind::DropShadow(s) = &e.kind {
                // Spread here (layers that aren't one outline) grows the pixels, like the renderer.
                let morph = if s.spread != 0.0 {
                    format!(
                        "<feMorphology in=\"{cast}\" operator=\"{}\" radius=\"{}\" result=\"s{i}m\"/>",
                        if s.spread > 0.0 { "dilate" } else { "erode" },
                        num(s.spread.abs())
                    )
                } else {
                    String::new()
                };
                let src = if morph.is_empty() { cast.to_string() } else { format!("s{i}m") };
                let _ = write!(
                    f,
                    "{morph}<feOffset in=\"{src}\" dx=\"{}\" dy=\"{}\"/><feGaussianBlur stdDeviation=\"{}\"/><feColorMatrix type=\"matrix\" values=\"0 0 0 0 {} 0 0 0 0 {} 0 0 0 0 {} 0 0 0 {} 0\" result=\"s{i}\"/>{}",
                    num(s.offset.0),
                    num(s.offset.1),
                    num(s.radius / 2.0),
                    num(f64::from(s.color.r)),
                    num(f64::from(s.color.g)),
                    num(f64::from(s.color.b)),
                    num(f64::from(s.color.a)),
                    if s.show_behind {
                        String::new()
                    } else {
                        format!("<feComposite in=\"s{i}\" in2=\"{knock}\" operator=\"out\" result=\"s{i}\"/>")
                    }
                );
                merge.push(format!("s{i}"));
            }
        }
        f.push_str("<feMerge>");
        for m in &merge {
            let _ = write!(f, "<feMergeNode in=\"{m}\"/>");
        }
        let _ = write!(f, "<feMergeNode in=\"{layer}\"/></feMerge></filter>");
        self.defs.push_str(&f);
        Some(fid)
    }

    fn node(&mut self, id: NodeId, xf: Affine, depth: usize) {
        if depth > MAX_EXPORT_DEPTH {
            return;
        }
        let Some(n) = self.doc.node(id) else { return };
        if !n.visible || (n.is_mask && self.mask != Some(id)) {
            return;
        }
        let xf = xf * n.transform;
        let id_attr = if self.opts.include_ids { format!(" id=\"{}\"", esc(&self.unique_id(&n.name))) } else { String::new() };
        if self.raster_whole(id, n, &id_attr) {
            return;
        }
        let mut attrs = id_attr;
        if n.opacity < 0.999 {
            let _ = write!(attrs, " opacity=\"{}\"", num(f64::from(n.opacity)));
        }
        let grains = self.grain_images(id, n);
        let blend = blend_style(n.blend);
        if !blend.is_empty() {
            attrs.push_str(&blend);
        } else if !grains.is_empty() || n.isolates_children() {
            // Grains blend with the layer only, like the canvas's isolated layer; a Normal group
            // blends its children among themselves first.
            attrs.push_str(" style=\"isolation:isolate\"");
        }
        let inherited = self.clip.clone();
        let spreads = n.effects.iter().any(|e| e.visible && matches!(&e.kind, EffectKind::DropShadow(s) if s.spread != 0.0));
        let sil = if spreads { figcraft_render::shadow::silhouette(self.doc, n) } else { None };
        if let Some(sil) = &sil {
            self.drop_shadows(n, sil, xf, &clip_attr(&inherited));
        }
        if let Some(fid) = self.filter_for(n, sil.is_some(), xf) {
            let _ = write!(attrs, " filter=\"url(#{fid})\"");
        }
        let leaf = matches!(n.kind, NodeKind::BooleanOp { .. }) || n.children.is_empty();
        let isolated = n.opacity < 0.999 || attrs.contains(" style=") || attrs.contains(" filter=") || n.effects.iter().any(|e| e.visible);
        let (paint_clip, kid_clip) = if isolated || leaf {
            attrs.push_str(&clip_attr(&inherited));
            (String::new(), None)
        } else {
            (clip_attr(&inherited), inherited.clone())
        };
        let _ = write!(self.body, "<g{attrs}>");
        if backdrop::has_effects(n)
            && let Some(img) = self.backdrop_image(id, None)
        {
            self.body.push_str(&img);
        }
        let (w, h) = (n.width, n.height);
        let outline = match &n.kind {
            NodeKind::Group | NodeKind::Page { .. } => None,
            _ => figcraft_render::node_outline(self.doc, n),
        };
        self.render_node(NodePaint { node: n, xf, depth, paint_clip, kid_clip, inherited, outline, grains, size: (w, h) });
        self.body.push_str("</g>");
    }

    fn render_node(&mut self, paint: NodePaint<'_>) {
        let NodePaint { node: n, xf, depth, paint_clip, kid_clip, inherited, outline, grains, size: (w, h) } = paint;
        let closed = figcraft_doc::geometry::is_closed(n) || matches!(n.kind, NodeKind::BooleanOp { .. } | NodeKind::Text(_));
        let live_text = if self.opts.outline_text { None } else { self.text_element(n, xf) };
        let run_paths = if live_text.is_none() { text_run_paths(n) } else { None };
        if let Some(t) = &live_text {
            self.body.push_str(t);
        } else if let Some(runs) = &run_paths {
            // Character ranges with their own fills: one path per style, like the canvas.
            for (path, fills) in runs {
                for p in fills {
                    if let Some(a) = self.paint_attr(p, w, h) {
                        let el = format!("<path transform=\"{}\" d=\"{}\" {a}/>", matrix(xf), path_d(path));
                        self.body.push_str(&clipped(&paint_clip, el));
                    }
                }
            }
        } else if let Some((path, rule)) = &outline
            && closed
        {
            if let NodeKind::Vector { network } = &n.kind
                && network.regions.iter().any(|r| r.fills.is_some() || r.paint_space.is_some())
            {
                let rounded = network.rounded();
                let net = rounded.as_ref().unwrap_or(network);
                for (i, region) in net.regions.iter().enumerate() {
                    let paints = region.fills.as_deref().unwrap_or(&n.fills);
                    let fr = if region.winding == figcraft_doc::WindingRule::EvenOdd { " fill-rule=\"evenodd\"" } else { "" };
                    if let Some(mut region_path) = net.region_path(i) {
                        let (pw, ph, frame) = region.paint_frame(w, h);
                        region_path.apply_affine(frame.inverse());
                        for p in paints {
                            if let Some(a) = self.paint_attr(p, pw, ph) {
                                let el = format!("<path transform=\"{}\" d=\"{}\" {a}{fr}/>", matrix(xf * frame), path_d(&region_path));
                                self.body.push_str(&clipped(&paint_clip, el));
                            }
                        }
                    }
                }
            } else {
                let fr = if *rule == figcraft_render::FillRule::EvenOdd { " fill-rule=\"evenodd\"" } else { "" };
                for p in &n.fills {
                    if let Some(a) = self.paint_attr(p, w, h) {
                        let el = format!("<path transform=\"{}\" d=\"{}\" {a}{fr}/>", matrix(xf), path_d(path));
                        self.body.push_str(&clipped(&paint_clip, el));
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
            self.body.push_str(&clipped(&paint_clip, simple));
        } else if n.has_visible_stroke() {
            self.stroke_paths(n, xf, outline.as_ref(), closed, &paint_clip);
        }
        self.body.push_str(&grains);
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
            let _ = write!(self.defs, "<clipPath id=\"{cid}\"{}><path d=\"{}\"/></clipPath>", clip_attr(&kid_clip), path_d(&cp));
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
                self.body.push_str("</g>");
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
            let (shape, cr) = match &outline {
                Some((mxf, (mp, rule))) => (
                    format!(" transform=\"{}\" d=\"{}\"", matrix(*mxf), path_d(mp)),
                    if *rule == figcraft_render::FillRule::EvenOdd { " clip-rule=\"evenodd\"" } else { "" },
                ),
                None => (" d=\"\"".to_string(), ""),
            };
            let _ = write!(self.defs, "<clipPath id=\"{cid}\"><path{shape}{cr}/></clipPath>");
            let _ = write!(self.body, "<g clip-path=\"url(#{cid})\">");
        } else {
            // The layer as drawn: its alpha or luminance masks.
            let mid = self.id("mask");
            let saved = std::mem::take(&mut self.body);
            let outer = self.mask.replace(c);
            let clip = self.clip.take();
            self.node(c, xf, depth + 1);
            self.clip = clip;
            self.mask = outer;
            let mask_body = std::mem::replace(&mut self.body, saved);
            let alpha = if kind == figcraft_doc::MaskType::Alpha { " style=\"mask-type:alpha\"" } else { "" };
            let _ = write!(self.defs, "<mask id=\"{mid}\" maskUnits=\"userSpaceOnUse\"{alpha}>{mask_body}</mask>");
            let _ = write!(self.body, "<g mask=\"url(#{mid})\">");
        }
    }

    /// The visible strokes of `n` as filled outline paths.
    fn stroke_paths(&mut self, n: &Node, xf: Affine, outline: Option<&(BezPath, figcraft_render::FillRule)>, closed: bool, paint_clip: &str) {
        let geom = match &n.kind {
            NodeKind::BooleanOp { .. } | NodeKind::Text(_) => outline.map(|(p, _)| p.clone()),
            _ => figcraft_doc::geometry::stroke_geometry(n),
        };
        if let Some(g) = geom {
            let area = figcraft_render::draw::stroke_area(n, &g, closed);
            let even = n.stroke.sides.is_some() || (n.stroke.dashes.is_empty() && closed && n.stroke.align != figcraft_doc::StrokeAlign::Center);
            let fr = if even { " fill-rule=\"evenodd\"" } else { "" };
            for p in &n.strokes {
                if let Some(a) = self.paint_attr(p, n.width, n.height) {
                    let el = format!("<path transform=\"{}\" d=\"{}\" {a}{fr}/>", matrix(xf), path_d(&area));
                    self.body.push_str(&clipped(paint_clip, el));
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
        let fill = n.fills.iter().find_map(|p| self.paint_attr(p, n.width, n.height)).unwrap_or_else(|| "fill=\"none\"".into());
        let chars: Vec<char> = t.characters.chars().collect();
        let mut spans = String::new();
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
                        let own = match layout.styles.get(style).and_then(|s| s.fills.as_ref()) {
                            Some(fills) => {
                                let a = fills.iter().find_map(|p| self.paint_attr(p, n.width, n.height)).unwrap_or_else(|| "fill=\"none\"".into());
                                format!(" {a}")
                            }
                            None => String::new(),
                        };
                        let _ = write!(spans, "<tspan x=\"{}\" y=\"{}\"{own}>{}</tspan>", num(glyphs[i].x), num(glyphs[i].y), esc(&text));
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
                let _ = write!(spans, "<tspan x=\"{}\" y=\"{}\">{}</tspan>", num(x), num(y), esc(&text));
            }
        }
        if spans.is_empty() {
            // No fonts laid it out: one line on the first baseline.
            let _ = write!(spans, "<tspan x=\"0\" y=\"{}\">{}</tspan>", num(st.font_size * 0.8), esc(&st.case.apply(&t.characters)));
        }
        let italic = if st.italic() { " font-style=\"italic\"" } else { "" };
        let spacing = match st.letter_spacing {
            figcraft_doc::LetterSpacing::Pixels(v) if v != 0.0 => format!(" letter-spacing=\"{}\"", num(v)),
            figcraft_doc::LetterSpacing::Percent(p) if p != 0.0 => format!(" letter-spacing=\"{}\"", num(p / 100.0 * st.font_size)),
            _ => String::new(),
        };
        Some(format!(
            "<text transform=\"{}\" font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\"{italic}{spacing} {fill} xml:space=\"preserve\">{spans}</text>",
            matrix(gxf),
            esc(&st.font_family),
            num(st.font_size),
            st.weight()
        ))
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
                    NodeKind::Ellipse { sweep, inner, .. } if inner == 0.0 && sweep >= std::f64::consts::TAU - 1e-9 => {
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
        let cap = match k.cap_start {
            StrokeCap::Round => " stroke-linecap=\"round\"",
            StrokeCap::Square => " stroke-linecap=\"square\"",
            _ => "",
        };
        let join = match k.join {
            StrokeJoin::Miter if (k.miter_limit - 4.0).abs() > 1e-9 => format!(" stroke-miterlimit=\"{}\"", num(k.miter_limit.max(1.0))),
            StrokeJoin::Miter => String::new(),
            StrokeJoin::Round => " stroke-linejoin=\"round\"".into(),
            StrokeJoin::Bevel => " stroke-linejoin=\"bevel\"".into(),
        };
        let dash = if k.dashes.is_empty() {
            String::new()
        } else {
            format!(" stroke-dasharray=\"{}\"", k.dashes.iter().map(|d| num(*d)).collect::<Vec<_>>().join(" "))
        };
        let mut out = String::new();
        for p in &n.strokes {
            if let Some(a) = self.paint_attr(p, n.width, n.height) {
                let a = a.replace("fill-opacity=", "stroke-opacity=").replacen("fill=", "stroke=", 1);
                let _ = write!(
                    out,
                    "<path transform=\"{}\" d=\"{}\" fill=\"none\" {a} stroke-width=\"{}\"{cap}{join}{dash}/>",
                    matrix(xf),
                    path_d(&path),
                    num(w)
                );
            }
        }
        Some(out)
    }

    /// `<image>` markup of `pix`, a render of `view`'s page area, with `attrs`; `None` when it is
    /// empty.
    fn image(&self, pix: &Pixmap, view: &View, attrs: &str) -> Option<String> {
        if pix.data().iter().skip(3).step_by(4).all(|a| *a == 0) {
            return None;
        }
        let png = figcraft_render::encode_png(pix).ok()?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);
        let t = self.page * Affine::translate(view.origin) * Affine::scale(1.0 / view.scale.max(1e-9));
        Some(format!(
            "<image transform=\"{}\" width=\"{}\" height=\"{}\"{attrs} href=\"data:image/png;base64,{b64}\"/>",
            matrix(t),
            pix.width(),
            pix.height()
        ))
    }

    /// Node `id`'s background blurs and glass as an image over its shape, drawn by the renderer
    /// from what lies under it among the exported layers (`opacity`: the layer's, when the image
    /// isn't inside its group).
    fn backdrop_image(&self, id: NodeId, opacity: Option<f32>) -> Option<String> {
        let view = raster_view(backdrop::shape_bounds(self.doc, id)?, RASTER_SCALE)?;
        let pix = backdrop::effects_beneath(self.doc, self.roots, id, &view, None)?;
        let op = opacity.filter(|o| *o < 0.999).map(|o| format!(" opacity=\"{}\"", num(f64::from(o)))).unwrap_or_default();
        self.image(&pix, &view, &op)
    }

    /// Layers whose look SVG can't build (texture scatters their pixels, a progressive blur varies
    /// across them, noise over a blur depends on their order) are drawn by the renderer and
    /// embedded whole, over their backdrop effects. False when the layer isn't one of them (or
    /// can't be rendered): it is written as vectors.
    fn raster_whole(&mut self, id: NodeId, n: &Node, id_attr: &str) -> bool {
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
        let Some(img) = figcraft_render::render_alone(self.doc, id, &view, Alone::WithEffects).and_then(|p| self.image(&p, &view, "")) else {
            return false;
        };
        let back = if backdrop::has_effects(n) { self.backdrop_image(id, Some(n.opacity.clamp(0.0, 1.0))) } else { None };
        let _ = write!(self.body, "<g{id_attr}{}{}>{}{img}</g>", blend_style(n.blend), clip_attr(&self.clip), back.unwrap_or_default());
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
            let blend = blend_style(nz.blend);
            let blend = blend.strip_prefix(" style=\"").and_then(|b| b.strip_suffix('"')).map(|b| format!(";{b}")).unwrap_or_default();
            let attrs = format!(" style=\"image-rendering:pixelated{blend}\"");
            if let Some(img) = figcraft_render::grain::grains(&layer, nz, xf, id.0 as u32).and_then(|g| self.image(&g, &view, &attrs)) {
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
            let mut s = String::new();
            for (p, rule) in outlines {
                let fr = if *rule == figcraft_render::FillRule::EvenOdd { " fill-rule=\"evenodd\"" } else { "" };
                let _ = write!(s, "<path transform=\"{}\" d=\"{}\" fill=\"{fill}\"{fr}/>", matrix(xf), path_d(p));
            }
            if let Some((c, half)) = stroke {
                for (p, _) in outlines {
                    let _ = write!(
                        s,
                        "<path transform=\"{}\" d=\"{}\" fill=\"none\" stroke=\"{c}\" stroke-width=\"{}\" stroke-linejoin=\"miter\" stroke-miterlimit=\"{}\"/>",
                        matrix(xf),
                        path_d(p),
                        num(half * 2.0),
                        num(SPREAD_MITER_LIMIT)
                    );
                }
            }
            s
        };
        let half = if spread.is_finite() { spread.abs() } else { 0.0 };
        if spread >= 0.0 || half <= 1e-6 {
            return paths(color, (half > 1e-6).then_some((color, half)));
        }
        let bounds =
            outlines.iter().map(|(p, _)| figcraft_geom::transform_rect_bbox(xf, figcraft_geom::Shape::bounding_box(p))).reduce(|a, b| a.union(b));
        let Some(b) = bounds.map(|b| b.inflate(1.0, 1.0)) else { return String::new() };
        let mid = self.id("mask");
        let _ = write!(
            self.defs,
            "<mask id=\"{mid}\" maskUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\">{}</mask>",
            num(b.x0),
            num(b.y0),
            num(b.width()),
            num(b.height()),
            paths("white", Some(("black", half)))
        );
        format!("<g mask=\"url(#{mid})\">{}</g>", paths(color, None))
    }

    /// A Gaussian blur filter over `region` (page units), cut out by `cover` (a filter input
    /// named `cover`, see [`W::cover_input`]); `None` for no blur and no cover.
    fn blur_filter(&mut self, radius: f64, region: Rect, cover: Option<&str>) -> Option<String> {
        let blur = radius.is_finite() && radius > 0.0;
        if !blur && cover.is_none() {
            return None;
        }
        let fid = self.id("filter");
        let _ = write!(
            self.defs,
            "<filter id=\"{fid}\" filterUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" color-interpolation-filters=\"sRGB\">",
            num(region.x0),
            num(region.y0),
            num(region.width()),
            num(region.height()),
        );
        if blur {
            let _ = write!(self.defs, "<feGaussianBlur stdDeviation=\"{}\" result=\"b\"/>", num(radius / 2.0));
        }
        if let Some(c) = cover {
            let _ = write!(self.defs, "{c}<feComposite in=\"{}\" in2=\"cover\" operator=\"out\"/>", if blur { "b" } else { "SourceGraphic" });
        }
        self.defs.push_str("</filter>");
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
        let mut img = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" width=\"{}\" height=\"{}\">",
            num(b.x0),
            num(b.y0),
            num(b.width()),
            num(b.height()),
            num(b.width()),
            num(b.height())
        );
        for (p, rule) in &cover {
            let fr = if *rule == figcraft_render::FillRule::EvenOdd { " fill-rule=\"evenodd\"" } else { "" };
            let _ = write!(img, "<path transform=\"{}\" d=\"{}\"{fr}/>", matrix(xf), path_d(p));
        }
        img.push_str("</svg>");
        let b64 = base64::engine::general_purpose::STANDARD.encode(img);
        Some(format!(
            "<feImage href=\"data:image/svg+xml;base64,{b64}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" result=\"cover\"/>",
            num(b.x0),
            num(b.y0),
            num(b.width()),
            num(b.height())
        ))
    }

    /// Drop shadows with spread, as geometry under the layer: its silhouette offset by the
    /// spread (exactly, like the renderer), moved, blurred and faded.
    fn drop_shadows(&mut self, n: &Node, sil: &[Outline], xf: Affine, clip: &str) {
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
            let reach = s.spread.abs() * SPREAD_MITER_LIMIT + s.radius.abs() * 1.5 + 2.0;
            let cover = if s.show_behind { None } else { self.cover_input(n, xf) };
            let filter =
                self.blur_filter(s.radius, b.inflate(reach, reach), cover.as_deref()).map(|f| format!(" filter=\"url(#{f})\"")).unwrap_or_default();
            let op = f64::from(s.color.a * n.opacity.clamp(0.0, 1.0));
            let blend = if matches!(s.blend, figcraft_color::BlendMode::Normal | figcraft_color::BlendMode::PassThrough) {
                String::new()
            } else {
                format!(" style=\"mix-blend-mode:{}\"", s.blend.css())
            };
            let body = self.spread_paths(sil, s.spread, at, &hex(s.color));
            let _ = write!(self.body, "<g opacity=\"{}\"{blend}{clip}><g{filter}>{body}</g></g>", num(op));
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
        let cr = if o.1 == figcraft_render::FillRule::EvenOdd { " clip-rule=\"evenodd\"" } else { "" };
        let cid = self.id("clip");
        let _ = write!(self.defs, "<clipPath id=\"{cid}\"><path transform=\"{}\" d=\"{}\"{cr}/></clipPath>", matrix(xf), path_d(&o.0));
        let hole = self.spread_paths(std::slice::from_ref(o), -s.spread, Affine::translate(s.offset) * xf, "black");
        let mid = self.id("mask");
        let rect = |fill: &str| {
            format!("<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{fill}\"/>", num(r.x0), num(r.y0), num(r.width()), num(r.height()))
        };
        let _ = write!(
            self.defs,
            "<mask id=\"{mid}\" maskUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\">{}{hole}</mask>",
            num(r.x0),
            num(r.y0),
            num(r.width()),
            num(r.height()),
            rect("white")
        );
        let filter = self.blur_filter(s.radius, r, None).map(|f| format!(" filter=\"url(#{f})\"")).unwrap_or_default();
        let _ = write!(
            self.body,
            "<g clip-path=\"url(#{cid})\"><g opacity=\"{}\"{filter}><g mask=\"url(#{mid})\">{}</g></g></g>",
            num(f64::from(s.color.a)),
            rect(&hex(s.color))
        );
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
        defs: String::new(),
        body: String::new(),
        next: 0,
        opts,
        mask: None,
        clip: None,
        used_ids: Default::default(),
        next_suffix: Default::default(),
    };
    for id in ids {
        let parent_xf = doc.node(*id).and_then(|n| n.parent).map(|p| doc.world_transform(p)).unwrap_or(Affine::IDENTITY);
        w.node(*id, origin * parent_xf, 0);
    }
    let (bw, bh) = (bounds.width().max(0.0), bounds.height().max(0.0));
    let defs = if w.defs.is_empty() { String::new() } else { format!("<defs>{}</defs>", w.defs) };
    Ok(format!(
        "<svg width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" fill=\"none\" xmlns=\"http://www.w3.org/2000/svg\">{}{}</svg>\n",
        num(bw),
        num(bh),
        num(bw),
        num(bh),
        w.body,
        defs
    ))
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
mod raster_fx_tests;
#[cfg(test)]
mod tests;
