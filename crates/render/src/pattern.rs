//! Pattern fills: the source layer drawn into one repeating cell at the resolution it shows at,
//! repeated by a tiling shader.

use std::cell::Cell;
use std::sync::Arc;

use figcraft_color::{PatternCell, PatternPaint};
use figcraft_doc::{Document, NodeId};
use figcraft_geom::Affine;
use tiny_skia::{FilterQuality, Pixmap, SpreadMode, Transform};

use crate::draw::{Bounds, Ctx, draw_node};
use crate::paint::Prepared;

/// The largest side of a cell's pixmap; a cell showing larger is drawn coarser.
const MAX_CELL_PX: f64 = 2048.0;
/// How deep patterns nest (a source filled with a pattern); deeper ones, and a source filled
/// with its own pattern, draw nothing.
pub const MAX_DEPTH: u32 = 2;

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Run `f` one pattern level deeper; `None` past [`MAX_DEPTH`].
pub fn nested<T>(f: impl FnOnce() -> Option<T>) -> Option<T> {
    let d = DEPTH.with(Cell::get);
    if d >= MAX_DEPTH {
        return None;
    }
    DEPTH.with(|c| c.set(d + 1));
    let out = f();
    DEPTH.with(|c| c.set(d));
    out
}

/// Pattern `pt` on a `w`×`h` box drawn into one cell at about `scale` device pixels per unit and
/// `time` seconds into animated GIFs:
/// the pixmap, the cell and the pixmap's pixels per unit on each axis (whole pixels per cell, so
/// the repeat lines up exactly).
pub fn cell_pixmap(doc: &Document, pt: &PatternPaint, w: f64, h: f64, scale: f64, time: f64) -> Option<(Pixmap, PatternCell, (f64, f64))> {
    let src = doc.node(NodeId(pt.source)).filter(|n| !n.is_page())?;
    let cell = pt.cell(src.width, src.height, w, h)?;
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let k = scale.min(MAX_CELL_PX / cell.size.0.max(cell.size.1));
    let (pw, ph) = ((cell.size.0 * k).ceil().clamp(1.0, MAX_CELL_PX), (cell.size.1 * k).ceil().clamp(1.0, MAX_CELL_PX));
    let (kx, ky) = (pw / cell.size.0, ph / cell.size.1);
    let mut pix = Pixmap::new(pw as u32, ph as u32)?;
    let bounds = Bounds::default();
    let ctx = Ctx {
        doc,
        scale: kx.max(ky) * cell.scale,
        outline_mode: false,
        outline: crate::Outline::default(),
        outlined: &[],
        skip: &[],
        lod: false,
        preview: false,
        magnified_image_pixels: false,
        under: None,
        time,
        bounds: &bounds,
        page_at: (0.0, 0.0),
        cache: None,
        images: 0,
        // A pattern's source is drawn into a small tile of its own: no canvas GPU there.
        accelerate: false,
    };
    let unrotated = src.transform.inverse();
    // The source is one level below the layer it fills, not a fresh render (#1082).
    let depth = crate::draw::layer_depth() + 1;
    nested(|| {
        for (tx, ty) in &cell.tiles {
            let xf = Affine::scale_non_uniform(kx, ky) * Affine::translate((*tx, *ty)) * Affine::scale(cell.scale) * unrotated;
            draw_node(&ctx, &mut pix, src.id, xf, depth);
        }
        Some(())
    })?;
    Some((pix, cell, (kx, ky)))
}

/// The shader for pattern `pt` on a `w`×`h` box shown at `scale` device pixels per unit.
pub fn prepare(doc: &Document, pt: &PatternPaint, w: f64, h: f64, scale: f64, time: f64, opacity: f32) -> Option<Prepared> {
    let (pix, cell, (kx, ky)) = cell_pixmap(doc, pt, w, h, scale, time)?;
    let transform = Transform::from_row((1.0 / kx) as f32, 0.0, 0.0, (1.0 / ky) as f32, cell.origin.0 as f32, cell.origin.1 as f32);
    Some(Prepared::Pattern { pixmap: Arc::new(pix), mode: SpreadMode::Repeat, quality: FilterQuality::Bilinear, opacity, transform })
}
