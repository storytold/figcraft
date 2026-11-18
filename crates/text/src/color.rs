//! Colour glyphs: embedded bitmap strikes (sbix, CBDT/EBDT) and COLR v0 layers.
//!
//! Colour emoji fonts don't draw like other glyphs: the picture itself lives in a bitmap strike
//! (sbix, CBDT/EBDT) or in a stack of coloured layers, not in the glyph's outline — in Apple Color
//! Emoji most emoji are `sbix` bitmaps while the flags are drawn from outlines, and Noto Color
//! Emoji has no outlines at all. This module gives the renderer either a
//! premultiplied RGBA raster at the size it asked for, or the layer glyphs and the palette colours
//! to fill with its own outlines. Anything else (COLR v1's paint graphs, alpha-only strikes) is
//! left to the outline pipeline, which draws whatever outlines the face has, as before.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::PoisonError;

use figcraft_geom::Affine;
use skrifa::bitmap::{BitmapData, BitmapGlyph, BitmapStrikes, Origin};
use skrifa::color::ColorPalettes;
use skrifa::instance::Size;
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId};

use crate::fontdb::{Coords, Face};

/// Colour rasters kept per face before the cache starts over, in bytes: they are pixels, and a
/// canvas zoomed far in makes each one large.
const MAX_RASTER_BYTES: usize = 32 * 1024 * 1024;
/// Rasters kept per face (misses count too: a face whose strikes cover a few glyphs builds one
/// per glyph and size, and the zoom changes the size).
const MAX_RASTER_ENTRIES: usize = 8192;
/// COLR layer lists kept per face (24 bytes a layer, so a count is enough).
const MAX_LAYERS: usize = 4096;
/// Most COLR layers read for one glyph.
const MAX_COLR_LAYERS: usize = 256;
/// Widest and tallest PNG strike decoded (strikes are at most a few hundred pixels).
const MAX_PNG_SIDE: u32 = 1024;
/// Largest colour raster per em (device pixels). Past this the renderer scales one up instead.
pub const MAX_PX_PER_EM: f64 = 512.0;

/// A colour bitmap glyph: premultiplied RGBA at `size` px per em, and the offset of its top-left
/// corner from the glyph's pen, in px at `size` (y down).
#[derive(Debug)]
pub struct ColorBitmap {
    pub width: u32,
    pub height: u32,
    pub left: f64,
    pub top: f64,
    pub rgba: Arc<[u8]>,
}

/// One layer of a COLR v0 glyph: the glyph that draws the layer and the palette colour it fills
/// with (straight RGBA: the renderer paints it).
#[derive(Debug)]
pub struct ColorLayer {
    pub gid: u32,
    pub rgba: [u8; 4],
}

/// A glyph's colour ink: what draws instead of its outline, placed in coordinates the caller
/// supplies (a bitmap pixel maps through `place`; COLR layers are drawn like the glyph's outline).
pub enum ColorInk {
    Bitmap(Arc<ColorBitmap>, Affine),
    Layers(Arc<Vec<ColorLayer>>),
}

/// Colour rasters cached per face, keyed by glyph, variations and size.
#[derive(Default)]
pub(crate) struct Rasters {
    entries: HashMap<(u32, Coords, u64), Option<Arc<ColorBitmap>>>,
    bytes: usize,
}

impl Rasters {
    fn get(&self, key: &(u32, Coords, u64)) -> Option<Arc<ColorBitmap>> {
        self.entries.get(key).and_then(Option::clone)
    }

    fn known(&self, key: &(u32, Coords, u64)) -> bool {
        self.entries.contains_key(key)
    }

    fn insert(&mut self, key: (u32, Coords, u64), bitmap: Option<Arc<ColorBitmap>>) {
        if let Some(b) = &bitmap {
            self.bytes += b.rgba.len();
        }
        if self.bytes > MAX_RASTER_BYTES || self.entries.len() > MAX_RASTER_ENTRIES {
            self.entries.clear();
            self.bytes = bitmap.as_ref().map(|b| b.rgba.len()).unwrap_or(0);
        }
        self.entries.insert(key, bitmap);
    }
}

impl Face {
    /// Whether the face has colour glyphs at all: bitmaps (sbix, CBDT/EBDT) or COLR layers.
    pub fn has_color(&self) -> bool {
        self.has_bitmaps() || self.has_colr()
    }

    /// The face has bitmap strikes (`sbix`/`CBDT`/`EBDT`)
    pub fn has_bitmaps(&self) -> bool {
        self.bitmap_strikes
    }

    /// The face has a `COLR` table.
    pub fn has_colr(&self) -> bool {
        self.colr
    }

    /// The glyph's embedded bitmap, scaled to `size` px per em (premultiplied RGBA), with the offset
    /// of its top-left corner from the pen in px at `size` (y down). `None` when the face has no
    /// bitmap for the glyph.
    pub fn color_bitmap(&self, gid: u32, coords: &Coords, size: f64) -> Option<Arc<ColorBitmap>> {
        if !self.bitmap_strikes || !size.is_finite() || size <= 0.0 {
            return None;
        }
        let size = size.clamp(1.0, MAX_PX_PER_EM);
        let key = (gid, coords.clone(), size.to_bits());
        let mut cache = self.rasters.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.known(&key) {
            return cache.get(&key);
        }
        let made = self.read_bitmap(gid, size).map(Arc::new);
        cache.insert(key, made.clone());
        made
    }

    /// The glyph's COLR v0 layers (empty when the glyph has none: COLR v1 glyphs, or no COLR data).
    pub fn color_layers(&self, gid: u32) -> Option<Arc<Vec<ColorLayer>>> {
        if !self.colr {
            return None;
        }
        let mut cache = self.layers.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(hit) = cache.get(&gid) {
            return hit.clone();
        }
        let made = self.read_layers(gid).map(Arc::new);
        if cache.len() >= MAX_LAYERS {
            cache.clear();
        }
        cache.insert(gid, made.clone());
        made
    }

    /// Read a strike bitmap and place it the way Skia's fontations port does
    /// (`src/ports/fontations/src/bitmap.rs`): the outer bearings are in font units and scale with
    /// the em, the inner ones are pixels at the strike's ppem, and an sbix image is measured from
    /// its lower-left corner (the spec places the graphic's origin at the glyph box's xMin, yMin).
    fn read_bitmap(&self, gid: u32, size: f64) -> Option<ColorBitmap> {
        let gid = GlyphId::new(gid);
        let font = FontRef::from_index(self.bytes.data(), self.index).ok()?;
        let strike = BitmapStrikes::new(&font).glyph_for_size(Size::new(size as f32), gid)?;
        let (rgba, width, height) = decode(&strike)?;
        if width == 0 || height == 0 || rgba.len() != width as usize * height as usize * 4 {
            return None;
        }
        let (left, top) = placement(&strike, size, self.upem);
        // Scale the strike's own pixels to the requested size (the caller blits them 1:1).
        let scale = size / f64::from(strike.ppem_y.max(1.0));
        let w = ((f64::from(width) * scale).round() as i64).clamp(1, MAX_PX_PER_EM as i64) as u32;
        let h = ((f64::from(height) * scale).round() as i64).clamp(1, MAX_PX_PER_EM as i64) as u32;
        let rgba = if (w, h) == (width, height) { rgba } else { scaled(&rgba, width, height, w, h)? };
        Some(ColorBitmap { width: w, height: h, left, top, rgba: rgba.into() })
    }

    /// Read the COLR v0 layers of a glyph (`None` for a glyph the table doesn't cover, which is
    /// every glyph of a COLR v1-only font).
    fn read_layers(&self, gid: u32) -> Option<Vec<ColorLayer>> {
        let font = FontRef::from_index(self.bytes.data(), self.index).ok()?;
        let colr = font.colr().ok()?;
        let range = colr.v0_base_glyph(GlyphId::new(gid))?;
        let palettes = ColorPalettes::new(&font);
        let palette = palettes.get(0);
        let colors = palette.as_ref().map(|p| p.colors()).unwrap_or(&[]);
        let mut out = Vec::new();
        for index in range {
            if out.len() >= MAX_COLR_LAYERS {
                break;
            }
            let (layer, color_index) = colr.v0_layer(index)?;
            let rgba = colors.get(usize::from(color_index)).map(|c| [c.red(), c.green(), c.blue(), c.alpha()]).unwrap_or([0, 0, 0, 255]);
            out.push(ColorLayer { gid: layer.to_u32(), rgba });
        }
        (!out.is_empty()).then_some(out)
    }
}

/// Where a strike's image sits from the pen: the image's top-left corner in px at `size` (y down).
fn placement(glyph: &BitmapGlyph, size: f64, upem: f64) -> (f64, f64) {
    let units = size / upem.max(1.0);
    let pixels = size / f64::from(glyph.ppem_y.max(1.0));
    let left = f64::from(glyph.inner_bearing_x) * pixels + f64::from(glyph.bearing_x) * units;
    let above = match glyph.placement_origin {
        // sbix: the origin is the image's lower-left corner, so the whole image hangs above it.
        Origin::BottomLeft => f64::from(glyph.inner_bearing_y) + f64::from(glyph.height),
        Origin::TopLeft => f64::from(glyph.inner_bearing_y),
    };
    (left, -above * pixels - f64::from(glyph.bearing_y) * units)
}

/// A strike's pixels as premultiplied RGBA (what the renderer's pixmaps hold), with their size.
fn decode(glyph: &BitmapGlyph<'_>) -> Option<(Vec<u8>, u32, u32)> {
    match &glyph.data {
        // The OpenType spec's PNG strikes are plain PNGs (straight alpha), so they are
        // premultiplied here for the renderer.
        BitmapData::Png(png) => {
            let (rgba, width, height) = decode_png(png)?;
            Some((premultiplied(rgba), width, height))
        }
        // Bit depth 32 without PNG is BGRA with premultiplied channels (the spec says so).
        BitmapData::Bgra(bgra) => {
            let (width, height) = (glyph.width, glyph.height);
            let mut rgba = bgra.get(..width as usize * height as usize * 4)?.to_vec();
            for px in rgba.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            Some((rgba, width, height))
        }
        // Alpha-only strikes would be masks, not colour: the outline pipeline draws those glyphs.
        BitmapData::Mask(_) => None,
    }
}

#[cfg(test)]
#[path = "color_tests.rs"]
mod tests;

/// A PNG strike as straight 8-bit RGBA.
fn decode_png(png: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    // Expand palettes, `tRNS` and sub-byte samples to 8-bit RGBA.
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    // The header's size is checked before anything is allocated for it: a font can claim any size.
    let (w, h) = reader.info().size();
    if w == 0 || h == 0 || w > MAX_PNG_SIDE || h > MAX_PNG_SIDE {
        return None;
    }
    let buf_size = reader.output_buffer_size()?;
    if buf_size > 4 * 1024 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; buf_size];
    let info = reader.next_frame(&mut buf).ok()?;
    let (width, height) = (info.width, info.height);
    let data = buf.get(..info.buffer_size())?;
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((rgba, width, height))
}

/// Straight RGBA → premultiplied RGBA (in place).
fn premultiplied(mut rgba: Vec<u8>) -> Vec<u8> {
    for px in rgba.chunks_exact_mut(4) {
        let a = u32::from(px[3]);
        for c in &mut px[..3] {
            *c = ((u32::from(*c) * a + 127) / 255) as u8;
        }
    }
    rgba
}

/// Premultiplied RGBA scaled to `dw`×`dh`: a box average when shrinking, bilinear when growing.
fn scaled(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Option<Vec<u8>> {
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return None;
    }
    let sample = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y.min(sh - 1) * sw + x.min(sw - 1)) * 4) as usize;
        match src.get(i..i + 4) {
            Some(px) => [px[0], px[1], px[2], px[3]],
            None => [0, 0, 0, 0],
        }
    };
    let mut out = vec![0u8; dw as usize * dh as usize * 4];
    if dw >= sw && dh >= sh {
        let at = |v: f64, n: u32| ((v + 0.5) * f64::from(n) - 0.5).clamp(0.0, f64::from(n - 1));
        for y in 0..dh {
            let sy = at(f64::from(y) / f64::from(dh), sh);
            let (y0, fy) = (sy.floor() as u32, sy.fract());
            for x in 0..dw {
                let sx = at(f64::from(x) / f64::from(dw), sw);
                let (x0, fx) = (sx.floor() as u32, sx.fract());
                let (p00, p10, p01, p11) = (sample(x0, y0), sample(x0 + 1, y0), sample(x0, y0 + 1), sample(x0 + 1, y0 + 1));
                let o = ((y * dw + x) * 4) as usize;
                for c in 0..4 {
                    let top = f64::from(p00[c]) * (1.0 - fx) + f64::from(p10[c]) * fx;
                    let bottom = f64::from(p01[c]) * (1.0 - fx) + f64::from(p11[c]) * fx;
                    out[o + c] = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        return Some(out);
    }
    let (fx, fy) = (f64::from(sw) / f64::from(dw), f64::from(sh) / f64::from(dh));
    for y in 0..dh {
        let (y0, y1) = ((f64::from(y) * fy).floor() as u32, ((f64::from(y) + 1.0) * fy).ceil() as u32);
        for x in 0..dw {
            let (x0, x1) = ((f64::from(x) * fx).floor() as u32, ((f64::from(x) + 1.0) * fx).ceil() as u32);
            let mut sum = [0.0f64; 4];
            let mut n = 0.0f64;
            for sy in y0..y1.clamp(y0 + 1, sh) {
                for sx in x0..x1.clamp(x0 + 1, sw) {
                    let px = sample(sx, sy);
                    for c in 0..4 {
                        sum[c] += f64::from(px[c]);
                    }
                    n += 1.0;
                }
            }
            let o = ((y * dw + x) * 4) as usize;
            for c in 0..4 {
                out[o + c] = (sum[c] / n.max(1.0)).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Some(out)
}
