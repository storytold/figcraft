//! Android Vector Drawable export: the exported layers as one `<vector>` XML drawable.
//!
//! Written from Android's public `VectorDrawable` documentation (elements `<vector>`, `<group>`,
//! `<path>`, `<clip-path>`, gradients as inline `GradientColor` resources through `aapt:attr`).
//! The drawable is as large as the exported area in dp (1 px = 1 dp), with a viewport of the same
//! size, so path coordinates are page units relative to the area's top-left corner.
//!
//! What maps exactly:
//! - shapes, booleans, vector networks, frames and text: their outlines
//!   (`figcraft_render::node_outline`, glyph outlines per styled run), with every transform baked
//!   into the path data (a `<group>` can only scale, rotate and translate, a `<path>` not at all);
//! - solid fills and paint, layer and stop opacity, folded into `#AARRGGBB` colours;
//! - centred, uniform, solid strokes as stroke attributes (width, cap, join, miter limit); every
//!   other stroke (inside, outside, dashed, variable width, arrows, per-side weights, gradient
//!   paints, under a skew or non-uniform scale) as its filled outline
//!   (`figcraft_render::draw::stroke_area`);
//! - linear, radial and angular (sweep) gradients, for any handle frame: a linear gradient's end
//!   point is projected so it bands along the width handle; radial and sweep gradients go in a
//!   `<group>` that scales and rotates the unit circle onto the handle ellipse (a 2×2 singular
//!   value decomposition; the remaining rotation turns a sweep's stops), with the path mapped back;
//! - clipping frames and vector masks, as a `<group>` with a `<clip-path>`.
//!
//! What can't map exactly depends on [`Compat`]: layer opacity over overlapping shapes, even-odd
//! fills and clip outlines, diamond gradients, alpha and luminance masks. Images, patterns,
//! effects and blend modes have no Vector Drawable equivalent and are left out (blend modes draw
//! as normal). Every change is listed in [`VdExport::warnings`] unless the mode is
//! [`Compat::Ignore`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod model;
mod paint;
mod stroke;
mod text;
mod walk;
mod warnings;
mod xml;

use figcraft_doc::{Document, NodeId};
use figcraft_geom::{Affine, Rect};
use figcraft_render::{ExportDepthError, check_export_depth};

/// How the export handles what a Vector Drawable can't draw exactly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compat {
    /// The closest drawing (layer opacity on each shape, even-odd fills and clip outlines rewritten as non-zero, diamond
    /// gradients as radial, alpha masks as vector masks), with a warning for each change.
    #[default]
    BestEffort,
    /// Never approximate: a layer or paint that can't be drawn exactly is left out, with a warning.
    Exact,
    /// Write what maps and leave out the rest without warnings (layer opacity over overlapping
    /// shapes is dropped, even-odd fills stay `fillType="evenOdd"`, even-odd clip outlines clip
    /// as non-zero, diamond gradients are left out, alpha masks clip to their outline).
    Ignore,
}

#[derive(Clone, Debug, Default)]
pub struct VdOptions {
    /// The page area the drawable covers (default: the exported layers' export bounds).
    pub bounds: Option<Rect>,
    /// Layer names as `android:name` attributes (unique in the file: repeats get `_2`, `_3`…).
    pub include_names: bool,
    pub compat: Compat,
}

/// A written drawable and what was changed or left out on the way.
#[derive(Clone, Debug, PartialEq)]
pub struct VdExport {
    pub xml: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum VdError {
    #[error("nothing to export")]
    Empty,
    #[error(transparent)]
    Depth(#[from] ExportDepthError),
}

/// The layers `ids` (bottom to top) as an Android Vector Drawable. Colours are written as they
/// are: convert a Display P3 document to sRGB first (`Document::in_srgb`).
pub fn try_export(doc: &Document, ids: &[NodeId], opts: &VdOptions) -> Result<VdExport, VdError> {
    check_export_depth(doc, ids)?;
    let area = match opts.bounds {
        Some(b) if [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) => b,
        _ => ids.iter().filter_map(|i| figcraft_render::export_bounds(doc, *i)).reduce(|a, b| a.union(b)).ok_or(VdError::Empty)?,
    };
    let (w, h) = (area.width(), area.height());
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return Err(VdError::Empty);
    }
    let origin = Affine::translate((-area.x0, -area.y0));
    let mut walker = walk::Walker::new(doc, opts);
    let mut items = Vec::new();
    for id in ids {
        let parent_xf = doc.node(*id).and_then(|n| n.parent).map(|p| doc.world_transform(p)).unwrap_or(Affine::IDENTITY);
        items.extend(walker.node(*id, origin * parent_xf, 0));
    }
    // Nothing drawn still writes a drawable Android loads (one empty path), so a batch export
    // keeps going, but say so.
    if items.is_empty() {
        let first = ids.first().and_then(|i| doc.node(*i)).map(|n| n.name.clone()).unwrap_or_default();
        walker.nothing_drawn(&first);
    }
    let xml = xml::document(w, h, &items);
    Ok(VdExport { xml, warnings: walker.finish() })
}

#[cfg(test)]
mod tests;
