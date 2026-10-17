//! The FigCraft renderer.
//!
//! [`render_view`] draws a page region at any zoom into a premultiplied RGBA pixmap (the canvas);
//! [`render_node`] rasterises one node for export at a scale. Everything is CPU (tiny-skia) so it
//! runs identically on desktop, in tests and in the browser.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod backdrop;
pub mod blur;
pub mod boolean;
pub mod draw;
mod export_depth;
pub mod fx;
pub mod glass;
pub mod grain;
pub mod image_profile;
pub mod images;
pub mod lod;
pub mod paint;
pub mod pattern;
pub mod retained;
pub mod shadow;

pub use draw::{node_outline, paint_bounds, stroke_fill, subtree_outline, ts_path};
pub use export_depth::{ExportDepthError, MAX_EXPORT_DEPTH, check_export_depth};
pub use retained::{Retained, Work};
pub use tiny_skia::{FillRule, Pixmap};

use figcraft_color::{Color, ColorProfile};
use figcraft_doc::{Document, ExportFormat, NodeId, NodeKind};
use figcraft_geom::{Affine, Rect};

/// Which part of a page to draw, and how big.
#[derive(Clone, Debug)]
pub struct View {
    /// Page coordinate shown at the pixmap's top-left.
    pub origin: (f64, f64),
    /// Device pixels per page unit.
    pub scale: f64,
    pub width: u32,
    pub height: u32,
}

impl View {
    pub fn transform(&self) -> Affine {
        Affine::scale(self.scale) * Affine::translate((-self.origin.0, -self.origin.1))
    }
    pub fn page_rect(&self) -> Rect {
        let s = self.scale.max(1e-9);
        Rect::new(self.origin.0, self.origin.1, self.origin.0 + f64::from(self.width) / s, self.origin.1 + f64::from(self.height) / s)
    }
}

/// Outline mode's options: what it draws besides the shapes' outlines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outline {
    /// Hidden layers are outlined too.
    pub hidden: bool,
    /// Every layer's box is drawn (groups, text, vectors: where their outline isn't the box).
    pub bounds: bool,
    /// Light lines, for a dark page (set by the renderer from the page's colour).
    pub light: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Fill the background (None = transparent, e.g. export without page colour).
    pub background: Option<Color>,
    pub outline_mode: bool,
    /// What outline mode draws besides the shapes' outlines.
    pub outline: Outline,
    /// Layers drawn in outline mode (with their contents) whatever `outline_mode` says.
    pub outlined: Vec<NodeId>,
    /// Nodes to leave out (an editor drawing them itself).
    pub skip: Vec<NodeId>,
    /// Draw only this layer of the page, where it is (Dev Mode's focus view).
    pub solo: Option<NodeId>,
    /// A preview (the canvas): when zoomed out, approximate what's too small on screen to see in
    /// detail, see [`lod`]. Off for everything that must be exact (exports, thumbnails); no
    /// effect in outline mode.
    pub lod: bool,
}

fn new_pixmap(w: u32, h: u32) -> Option<Pixmap> {
    let w = w.clamp(1, 16_384);
    let h = h.clamp(1, 16_384);
    Pixmap::new(w, h)
}

/// The largest backdrop radius (background blur, glass: see [`figcraft_doc::Effect::backdrop_radius`])
/// in the document (0 without any).
pub(crate) fn max_backdrop_blur(doc: &Document) -> f64 {
    doc.max_backdrop_radius()
}

/// How far (device pixels) a background blur of `radius` reads its backdrop at `scale`.
pub(crate) fn backdrop_reach(radius: f64, scale: f64) -> f64 {
    // The three box passes of a blur with sigma = radius / 2 reach at most 3 sigma + 3 pixels.
    if radius > 0.0 { radius * 1.5 * scale + 3.0 } else { 0.0 }
}

/// Render a page region, splitting it into horizontal bands rendered in parallel (desktop).
pub fn render_view(doc: &Document, page: NodeId, view: &View, opts: &Options) -> Option<Pixmap> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let threads = rayon::current_num_threads().clamp(1, 16) as u32;
        let bands = threads.min(view.height / 96).max(1);
        if bands > 1
            && view.width.saturating_mul(view.height) > 300_000
            && let Some(p) = render_bands(doc, page, view, opts, bands)
        {
            return Some(p);
        }
    }
    render_view_single(doc, page, view, opts)
}

/// Render `view` as `bands` horizontal bands in parallel. Bands overlap by what background blurs
/// read and each keeps only its own rows, so a backdrop never ends at a band's edge. `None` when a
/// blur reaches past a whole band (every band would render most of the view: render it in one
/// piece instead).
#[cfg(not(target_arch = "wasm32"))]
fn render_bands(doc: &Document, page: NodeId, view: &View, opts: &Options, bands: u32) -> Option<Pixmap> {
    use rayon::prelude::*;
    let band_h = view.height.div_ceil(bands.max(1));
    let overlap = backdrop_reach(max_backdrop_blur(doc), view.scale).ceil();
    if !overlap.is_finite() || overlap > f64::from(band_h) {
        return None;
    }
    let overlap = overlap as u32;
    let s = view.scale.max(1e-9);
    let parts: Vec<(u32, Option<Pixmap>)> = (0..bands)
        .into_par_iter()
        .map(|i| {
            let y0 = i * band_h;
            let h = band_h.min(view.height.saturating_sub(y0)).max(1);
            let top = y0.saturating_sub(overlap);
            let bottom = (y0 + h + overlap).min(view.height).max(y0 + h);
            let v = View { origin: (view.origin.0, view.origin.1 + f64::from(top) / s), scale: view.scale, width: view.width, height: bottom - top };
            let part = render_view_single(doc, page, &v, opts);
            let own = tiny_skia::IntRect::from_xywh(0, (y0 - top) as i32, view.width.clamp(1, 16_384), h);
            (y0, if overlap == 0 { part } else { part.zip(own).and_then(|(p, r)| p.clone_rect(r)) })
        })
        .collect();
    let mut out = new_pixmap(view.width, view.height)?;
    let row = out.width() as usize * 4;
    for (y0, part) in parts {
        let Some(part) = part else { continue };
        // Bands are as wide as the view: their rows go straight into place.
        let start = y0 as usize * row;
        let dst = out.data_mut();
        let n = part.data().len().min(dst.len().saturating_sub(start));
        if part.width() == view.width.clamp(1, 16_384)
            && let (Some(d), Some(s)) = (dst.get_mut(start..start + n), part.data().get(..n))
        {
            d.copy_from_slice(s);
        }
    }
    Some(out)
}

/// Render a page region on the calling thread.
pub fn render_view_single(doc: &Document, page: NodeId, view: &View, opts: &Options) -> Option<Pixmap> {
    let mut pix = new_pixmap(view.width, view.height)?;
    let bg = opts.background.or_else(|| match doc.node(page).map(|p| &p.kind) {
        Some(NodeKind::Page { background, .. }) => Some(*background),
        _ => None,
    });
    if let Some(bg) = bg {
        pix.fill(paint::ts_color(bg));
    }
    let lod = opts.lod && !opts.outline_mode && view.scale <= lod::MAX_SCALE;
    let bounds = draw::Bounds::default();
    let ctx = draw::Ctx {
        doc,
        scale: view.scale,
        outline_mode: opts.outline_mode,
        outline: Outline { light: bg.is_some_and(Color::is_dark), ..opts.outline },
        outlined: &opts.outlined,
        skip: &opts.skip,
        lod,
        under: None,
        time: 0.0,
        bounds: &bounds,
    };
    let xf = view.transform();
    let Some(p) = doc.node(page) else { return Some(pix) };
    match opts.solo.filter(|id| *id != page && doc.page_of(*id) == Some(page)) {
        Some(id) => draw::draw_node(&ctx, &mut pix, id, xf * parent_transform(doc, id), 0),
        None => {
            for c in &p.children {
                draw::draw_node(&ctx, &mut pix, *c, xf, 0);
            }
        }
    }
    Some(pix)
}

/// Render several nodes (in z-order) into an existing pixmap through `view`.
pub fn render_nodes_into(doc: &Document, ids: &[NodeId], view: &View, pix: &mut Pixmap) {
    nodes_into(doc, ids, view, pix, 0.0);
}

/// [`render_nodes_into`] `time` seconds into animated GIFs.
fn nodes_into(doc: &Document, ids: &[NodeId], view: &View, pix: &mut Pixmap, time: f64) {
    let bounds = draw::Bounds::default();
    let ctx = draw::Ctx {
        doc,
        scale: view.scale,
        outline_mode: false,
        outline: Outline::default(),
        outlined: &[],
        skip: &[],
        lod: false,
        under: None,
        time,
        bounds: &bounds,
    };
    for id in ids {
        draw::draw_node(&ctx, pix, *id, view.transform() * parent_transform(doc, *id), 0);
    }
}

/// The page transform of layer `id`'s parent.
fn parent_transform(doc: &Document, id: NodeId) -> Affine {
    doc.node(id).and_then(|n| n.parent).map(|p| doc.world_transform(p)).unwrap_or(Affine::IDENTITY)
}

/// How an export draws a node by itself (see [`render_alone`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alone {
    /// With its effects, opacity and blend, but its backdrop effects (background blur and glass
    /// read what lies under it: see [`backdrop::effects_beneath`]).
    WithEffects,
    /// Its content only: no effects, opacity or blend.
    Plain,
}

/// Node `id` drawn by itself into `view` (a mask layer as a plain layer), for exports that draw
/// what vectors can't (blurs, grain, shadows) as images.
pub fn render_alone(doc: &Document, id: NodeId, view: &View, how: Alone) -> Option<Pixmap> {
    let mut tmp = doc.clone();
    let n = tmp.node_mut(id)?;
    n.is_mask = false;
    match how {
        Alone::WithEffects => {
            n.effects.retain(|e| !matches!(e.kind, figcraft_doc::EffectKind::BackgroundBlur(_) | figcraft_doc::EffectKind::Glass(_)))
        }
        Alone::Plain => {
            n.effects.clear();
            n.opacity = 1.0;
            n.blend = figcraft_color::BlendMode::Normal;
        }
    }
    let mut pix = Pixmap::new(view.width.clamp(1, 16_384), view.height.clamp(1, 16_384))?;
    render_nodes_into(&tmp, &[id], view, &mut pix);
    Some(pix)
}

/// The page area node `id` paints into with its effects (blurs, shadows and texture reach past
/// its bounds): what an export rasterises of it.
pub fn raster_bounds(doc: &Document, id: NodeId) -> Option<Rect> {
    let n = doc.node(id)?;
    let sc = doc.world_transform(id).determinant().abs().sqrt();
    let sc = if sc.is_finite() && sc > 0.0 { sc } else { 1.0 };
    let m = draw::effect_margin(n).abs() * sc;
    let m = if m.is_finite() { m } else { 0.0 };
    Some(doc.world_render_bounds(id)?.inflate(m, m))
}

/// The page-space rectangle a node exports: a clipping frame's box (plus its own effects), else
/// everything it paints.
pub fn export_bounds(doc: &Document, id: NodeId) -> Option<Rect> {
    doc.world_render_bounds(id)
}

/// Rasterise one node at `scale` (export). Transparent background.
pub fn render_node(doc: &Document, id: NodeId, scale: f64) -> Option<Pixmap> {
    render_node_at(doc, id, scale, 0.0)
}

/// [`render_node`] `time` seconds into animated GIFs (the presenter plays them).
pub fn render_node_at(doc: &Document, id: NodeId, scale: f64, time: f64) -> Option<Pixmap> {
    area_at(doc, &[id], export_bounds(doc, id)?, scale, time)
}

/// Rasterise `ids` (bottom to top) over page area `b` at `scale` (export): the image starts at the
/// area's top-left and its sides round up to whole pixels. `None` past 16384 pixels a side.
pub fn render_area(doc: &Document, ids: &[NodeId], b: Rect, scale: f64) -> Option<Pixmap> {
    area_at(doc, ids, b, scale, 0.0)
}

fn area_at(doc: &Document, ids: &[NodeId], b: Rect, scale: f64, time: f64) -> Option<Pixmap> {
    let scale = if scale.is_finite() && scale > 0.0 { scale.min(64.0) } else { 1.0 };
    let w = (b.width() * scale - 1e-6).ceil().max(1.0);
    let h = (b.height() * scale - 1e-6).ceil().max(1.0);
    if !(w <= 16_384.0 && h <= 16_384.0 && b.x0.is_finite() && b.y0.is_finite()) {
        return None;
    }
    let view = View { origin: (b.x0, b.y0), scale, width: w as u32, height: h as u32 };
    let mut pix = new_pixmap(view.width, view.height)?;
    nodes_into(doc, ids, &view, &mut pix, time);
    Some(pix)
}

/// Un-premultiply into straight RGBA8 bytes.
pub fn to_rgba8(p: &Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(p.data().len());
    for px in p.data().as_chunks::<4>().0 {
        let a = px.get(3).copied().unwrap_or(0);
        if a == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            let un = |c: u8| ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
            out.extend_from_slice(&[
                un(px.first().copied().unwrap_or(0)),
                un(px.get(1).copied().unwrap_or(0)),
                un(px.get(2).copied().unwrap_or(0)),
                a,
            ]);
        }
    }
    out
}

/// Bring a render of a document in `profile` into sRGB, the space the screen and the system
/// clipboard expect (out-of-gamut colours clip). A no-op for sRGB documents.
pub fn to_srgb(pix: &mut Pixmap, profile: ColorProfile) {
    profile.conversion_to(ColorProfile::Srgb).apply_pixels(pix.data_mut());
}

/// Write `pixels` with `enc`, embedding `icc` when given.
fn write_encoded(
    mut enc: impl image::ImageEncoder,
    pixels: &[u8],
    p: &Pixmap,
    kind: image::ExtendedColorType,
    icc: Option<Vec<u8>>,
) -> Result<(), String> {
    if let Some(icc) = icc {
        enc.set_icc_profile(icc).map_err(|e| e.to_string())?;
    }
    enc.write_image(pixels, p.width(), p.height(), kind).map_err(|e| e.to_string())
}

/// Encode as PNG, with an `iCCP` chunk when `icc` is given.
fn png_with(p: &Pixmap, icc: Option<Vec<u8>>) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    write_encoded(image::codecs::png::PngEncoder::new(&mut out), &to_rgba8(p), p, image::ExtendedColorType::Rgba8, icc)?;
    Ok(out)
}

/// Encode as JPEG over `background` (JPEG has no alpha), with an ICC profile when given.
fn jpeg_with(p: &Pixmap, background: Color, quality: u8, icc: Option<Vec<u8>>) -> Result<Vec<u8>, String> {
    let rgba = to_rgba8(p);
    let bg = background.to_rgba8();
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for px in rgba.as_chunks::<4>().0 {
        let a = u32::from(px.get(3).copied().unwrap_or(0));
        for c in 0..3 {
            let s = u32::from(px.get(c).copied().unwrap_or(0));
            let b = u32::from(bg.get(c).copied().unwrap_or(255));
            rgb.push(((s * a + b * (255 - a) + 127) / 255) as u8);
        }
    }
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100));
    write_encoded(enc, &rgb, p, image::ExtendedColorType::Rgb8, icc)?;
    Ok(out)
}

/// Encode as lossless WebP, with an ICC profile when given.
fn webp_with(p: &Pixmap, icc: Option<Vec<u8>>) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    write_encoded(image::codecs::webp::WebPEncoder::new_lossless(&mut out), &to_rgba8(p), p, image::ExtendedColorType::Rgba8, icc)?;
    Ok(out)
}

/// Encode as PNG (untagged).
pub fn encode_png(p: &Pixmap) -> Result<Vec<u8>, String> {
    png_with(p, None)
}

/// Encode as JPEG over `background` (JPEG has no alpha).
pub fn encode_jpeg(p: &Pixmap, background: Color, quality: u8) -> Result<Vec<u8>, String> {
    jpeg_with(p, background, quality, None)
}

/// Encode as lossless WebP.
pub fn encode_webp(p: &Pixmap) -> Result<Vec<u8>, String> {
    webp_with(p, None)
}

/// Encode an export in `format` (PNG, JPEG over white, WebP) tagged with the document's colour
/// `profile`. An sRGB PNG gets an `sRGB` chunk; a Display P3 PNG gets the profile as `iCCP` plus
/// `cICP` code points; Display P3 JPEG and WebP files embed the ICC profile (untagged files are read
/// as sRGB). SVG and PDF are not pixel formats: an error.
pub fn encode_export(p: &Pixmap, format: ExportFormat, profile: ColorProfile) -> Result<Vec<u8>, String> {
    let icc = (!profile.is_srgb()).then(|| profile.icc());
    match format {
        ExportFormat::Png => {
            let png = png_with(p, icc)?;
            Ok(if profile.is_srgb() { png_insert_chunk(png, b"sRGB", &[0]) } else { png_insert_chunk(png, b"cICP", &profile.cicp()) })
        }
        ExportFormat::Jpg => jpeg_with(p, Color::WHITE, 90, icc),
        ExportFormat::Webp => webp_with(p, icc),
        ExportFormat::Svg | ExportFormat::Pdf => Err(format!("{} is not a pixel format", format.label())),
    }
}

/// Insert a chunk right after a PNG's `IHDR` (where colour-space chunks go: before `PLTE` and
/// `IDAT`). Anything that isn't a PNG comes back unchanged.
pub fn png_insert_chunk(png: Vec<u8>, kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    const IHDR_END: usize = 8 + 8 + 13 + 4;
    let (Some(head), Some(rest)) = (png.get(..IHDR_END), png.get(IHDR_END..)) else { return png };
    if head.get(12..16) != Some(b"IHDR".as_slice()) {
        return png;
    }
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    let mut out = Vec::with_capacity(png.len() + 12 + data.len());
    out.extend_from_slice(head);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
    out.extend_from_slice(rest);
    out
}

#[cfg(test)]
mod backdrop_shadow_tests;
#[cfg(test)]
mod backdrop_tests;
#[cfg(test)]
mod band_tests;
#[cfg(test)]
mod boolean_cache_tests;
#[cfg(test)]
mod bounds_memo_tests;
#[cfg(test)]
mod effect_order_tests;
#[cfg(test)]
mod effects_behaviour_tests;
#[cfg(test)]
mod gif_tests;
#[cfg(test)]
mod glass_bench_tests;
#[cfg(test)]
mod glass_cut_tests;
#[cfg(test)]
mod grain_tests;
#[cfg(test)]
mod group_effects_tests;
#[cfg(test)]
mod image_budget_tests;
#[cfg(test)]
mod image_profile_tests;
#[cfg(test)]
mod lod_tests;
#[cfg(test)]
mod paint_behaviour_tests;
#[cfg(test)]
mod progressive_tests;
#[cfg(test)]
mod retained_edit_tests;
#[cfg(test)]
mod retained_tests;
#[cfg(test)]
mod rtl_list_tests;
#[cfg(test)]
mod solo_tests;
#[cfg(test)]
mod spread_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod width_tests;
