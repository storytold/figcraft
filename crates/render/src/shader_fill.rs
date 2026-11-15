//! Procedural shader fills ([`figcraft_color::ShaderPaint`]): the field sampled into a pixmap
//! over the filled box at about the resolution it shows at, drawn by a pattern shader.
//!
//! The pixmap is at most [`MAX_SIDE`] pixels a side ([`PREVIEW_SIDE`] on the canvas), so a large
//! box draws from a coarser one, and pixmaps are kept in a small cache: the field costs a few
//! dozen hashes per pixel, too much to work out again on every frame. Outside the box (a stroke
//! on its outside, say) the pixmap is mirrored, which continues the pattern without a seam but
//! isn't the field itself there.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use figcraft_color::ShaderPaint;
use tiny_skia::{FilterQuality, Pixmap, SpreadMode, Transform};

use crate::paint::Prepared;

/// The largest side of a shader pixmap in exports and other full renders.
pub const MAX_SIDE: f64 = 4096.0;
/// The largest side of a shader pixmap on the canvas.
pub const PREVIEW_SIDE: f64 = 1024.0;
/// The most bytes of pixels the cache holds (one larger pixmap is held alone) and the most
/// pixmaps.
const MAX_CACHED_BYTES: u64 = 128 << 20;
const MAX_CACHED: usize = 64;

/// Everything a pixmap depends on, floats by their bits.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    pattern: u8,
    shape: u8,
    bands: u8,
    colors: [u32; 8],
    numbers: [u64; 4],
    grain: u32,
    seed: u32,
    size: (u32, u32),
}

#[derive(Default)]
struct Cache {
    map: HashMap<Key, (Arc<Pixmap>, u64)>,
    bytes: u64,
    tick: u64,
}

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Cache::default()))
}

/// The pixel size for a `w`×`h` box at `scale` device pixels per unit. The scale is rounded up to
/// a power of two, so zooming works a pixmap out once per doubling and not on every step.
pub(crate) fn pixel_size(w: f64, h: f64, scale: f64, preview: bool) -> Option<(u32, u32)> {
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    let scale = if scale.is_finite() && scale > 0.0 { scale.clamp(1.0 / 64.0, 64.0) } else { 1.0 };
    let max = if preview { PREVIEW_SIDE } else { MAX_SIDE };
    let k = 2f64.powi(scale.log2().ceil() as i32).min(max / w.max(h));
    Some(((w * k).ceil().clamp(1.0, max) as u32, (h * k).ceil().clamp(1.0, max) as u32))
}

/// Shader `sp` over a `w`×`h` box (document units) as premultiplied pixels, shown at about
/// `scale` device pixels per unit. `None` for a box without a size.
pub fn pixmap(sp: &ShaderPaint, w: f64, h: f64, scale: f64, preview: bool) -> Option<Arc<Pixmap>> {
    let (pw, ph) = pixel_size(w, h, scale, preview)?;
    let sp = sp.sanitized();
    let rgba = |c: figcraft_color::Color| [c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits()];
    let (a, b) = (rgba(sp.color_a), rgba(sp.color_b));
    let key = Key {
        pattern: sp.pattern as u8,
        shape: sp.shape as u8,
        bands: sp.bands,
        colors: [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]],
        numbers: [sp.scale.to_bits(), sp.time.to_bits(), w.to_bits(), h.to_bits()],
        grain: sp.grain.to_bits(),
        seed: sp.seed,
        size: (pw, ph),
    };
    {
        let mut c = cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        c.tick = c.tick.wrapping_add(1);
        let tick = c.tick;
        if let Some((p, used)) = c.map.get_mut(&key) {
            *used = tick;
            return Some(p.clone());
        }
    }
    let mut pix = Pixmap::new(pw, ph)?;
    let sampler = sp.sampler();
    let (ux, uy) = (w / f64::from(pw), h / f64::from(ph));
    for (j, row) in pix.data_mut().chunks_exact_mut(pw as usize * 4).enumerate() {
        let y = (j as f64 + 0.5) * uy;
        for (i, px) in row.chunks_exact_mut(4).enumerate() {
            let x = (i as f64 + 0.5) * ux;
            let c = sampler.color_in(x, y, w, h);
            let q = |v: f32| (v * c.a * 255.0).round().clamp(0.0, 255.0) as u8;
            px.copy_from_slice(&[q(c.r), q(c.g), q(c.b), (c.a * 255.0).round().clamp(0.0, 255.0) as u8]);
        }
    }
    let pix = Arc::new(pix);
    let size = pix.data().len() as u64;
    let mut c = cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    while !c.map.is_empty() && (c.bytes.saturating_add(size) > MAX_CACHED_BYTES || c.map.len() >= MAX_CACHED) {
        let Some(oldest) = c.map.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone()) else { break };
        if let Some((p, _)) = c.map.remove(&oldest) {
            c.bytes = c.bytes.saturating_sub(p.data().len() as u64);
        }
    }
    c.tick = c.tick.wrapping_add(1);
    let tick = c.tick;
    if c.map.insert(key, (pix.clone(), tick)).is_none() {
        c.bytes = c.bytes.saturating_add(size);
    }
    Some(pix)
}

/// The shader for `sp` on a `w`×`h` box shown at `scale` device pixels per unit.
pub fn prepare(sp: &ShaderPaint, w: f64, h: f64, scale: f64, preview: bool, opacity: f32) -> Option<Prepared> {
    let pixmap = pixmap(sp, w, h, scale, preview)?;
    let transform = Transform::from_scale((w / f64::from(pixmap.width())) as f32, (h / f64::from(pixmap.height())) as f32);
    Some(Prepared::Pattern { pixmap, mode: SpreadMode::Reflect, quality: FilterQuality::Bilinear, opacity, transform })
}
