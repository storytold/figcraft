//! Strokes Android can draw itself, and the rule for the filled outline of the rest.

use figcraft_doc::{Node, NodeKind, StrokeAlign, StrokeCap, StrokeJoin};
use figcraft_geom::{Affine, BezPath};

/// A stroke's Vector Drawable attributes: its path (local), width (viewport), cap, join, miter.
pub(crate) struct Native {
    pub path: BezPath,
    pub width: f64,
    pub cap: &'static str,
    pub join: &'static str,
    pub miter: f64,
}

/// Whether `xf` keeps angles and scales evenly (a rotation, flip and uniform scale), so a stroke
/// width stays one width.
pub(crate) fn similarity(xf: Affine) -> bool {
    let [a, b, c, d, _, _] = xf.as_coeffs();
    let s2 = (a * a + b * b + c * c + d * d) / 2.0;
    s2.is_finite() && s2 > 0.0 && (a * c + b * d).abs() <= 1e-6 * s2 && (a * a + b * b - c * c - d * d).abs() <= 1e-6 * s2
}

/// `n`'s stroke as stroke attributes when Android draws it the same: centred, uniform, without
/// dashes, arrows or per-point styles, on a shape (not text or a boolean) drawn through a
/// similarity `xf`. `None`: write its filled outline.
pub(crate) fn native(n: &Node, xf: Affine) -> Option<Native> {
    let k = &n.stroke;
    if k.align != StrokeAlign::Center
        || k.sides.is_some()
        || k.profile().is_some()
        || k.brush.is_some()
        || !k.dashes.is_empty()
        || k.cap_start != k.cap_end
        || k.cap_start.is_arrow()
        || !(k.weight.is_finite() && k.weight > 0.0)
        || matches!(n.kind, NodeKind::Text(_) | NodeKind::BooleanOp { .. } | NodeKind::Group | NodeKind::Page { .. })
        || matches!(&n.kind, NodeKind::Vector { network } if network.has_point_styles(k.join))
        || !similarity(xf)
    {
        return None;
    }
    let path = figcraft_doc::geometry::stroke_geometry(n)?;
    let width = k.weight * xf.determinant().abs().sqrt();
    let cap = match k.cap_start {
        StrokeCap::Round => "round",
        StrokeCap::Square => "square",
        _ => "butt",
    };
    let join = match k.join {
        StrokeJoin::Miter => "miter",
        StrokeJoin::Round => "round",
        StrokeJoin::Bevel => "bevel",
    };
    let miter = if k.miter_limit.is_finite() { k.miter_limit.max(1.0) } else { 4.0 };
    width.is_finite().then_some(Native { path, width, cap, join, miter })
}

/// Whether `n`'s filled stroke outline needs the even-odd rule (rings of inside and outside
/// strokes, per-side strokes), as the renderer fills it.
pub(crate) fn outline_even_odd(n: &Node, closed: bool) -> bool {
    n.stroke.sides.is_some() || (n.stroke.dashes.is_empty() && closed && n.stroke.align != StrokeAlign::Center)
}
