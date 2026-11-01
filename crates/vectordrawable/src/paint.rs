//! Gradients in Vector Drawable terms, and the even-odd → non-zero rewrite.
//!
//! A gradient's handles map the unit box onto the layer: (0,0) → from, (1,0) → to, (0,1) →
//! width, a frame `F` (with the layer's transform, into the viewport). A linear gradient bands
//! along `F`'s second axis, so its Vector Drawable end point is the first axis with the part
//! along the second removed: exact for any frame, skewed too. Radial and sweep gradients live on
//! the unit circle: `F = T · R(φ) · S(sx, sy) · R(θ)` (a 2×2 singular value decomposition; `sy`
//! is negative for a mirrored frame), and a circle doesn't change under the last rotation, so a
//! `<group>` with translate `T`, rotation `φ` and scale `S` holds a unit gradient at the origin
//! and the path mapped back by the group's inverse. A sweep turns with `θ`: its stops are
//! shifted by `θ / 2π` instead.

use figcraft_color::{Color, Gradient};
use figcraft_geom::{Affine, BezPath, Vec2};

use crate::model::{Grad, GradKind, GroupXf};

/// A gradient fill, ready to write.
#[derive(Debug, PartialEq)]
pub(crate) enum GradOut {
    /// In viewport coordinates, on the path as it is.
    Plain(Grad),
    /// In the coordinates of a `<group>` with this transform; `inverse` maps the path into them.
    Grouped { xf: GroupXf, inverse: Affine, grad: Grad },
    /// One stop: a solid colour.
    Solid(Color),
    /// A frame that collapses to a line or a point: the renderer paints nothing, nor do we.
    Nothing,
}

/// The gradient handle frame in a `w`×`h` layer's local space.
pub(crate) fn frame(gradient: &Gradient, width: f64, height: f64) -> Affine {
    let start = (gradient.from.0 * width, gradient.from.1 * height);
    let end = (gradient.to.0 * width, gradient.to.1 * height);
    let width_handle = (gradient.width.0 * width, gradient.width.1 * height);
    Affine::new([end.0 - start.0, end.1 - start.1, width_handle.0 - start.0, width_handle.1 - start.1, start.0, start.1])
}

/// `m` (2×2, column vectors `(a, b)` and `(c, d)`) as `R(φ) · diag(sx, sy) · R(θ)`: `(φ, sx,
/// sy, θ)` in radians. `sx ≥ |sy|`; `sy < 0` when `m` mirrors.
pub(crate) fn svd(m00: f64, m01: f64, m10: f64, m11: f64) -> (f64, f64, f64, f64) {
    let (sum, difference, off_sum, off_difference) = (f64::midpoint(m00, m11), (m00 - m11) / 2.0, f64::midpoint(m01, m10), (m01 - m10) / 2.0);
    let (scale_sum, scale_difference) = (sum.hypot(off_difference), difference.hypot(off_sum));
    let (angle1, angle2) = (off_sum.atan2(difference), off_difference.atan2(sum));
    (f64::midpoint(angle2, angle1), scale_sum + scale_difference, scale_sum - scale_difference, (angle2 - angle1) / 2.0)
}

/// The colour of `stops` (sorted, at least one) at `t`, clamped at the ends.
#[expect(clippy::cast_possible_truncation, reason = "Interpolation deliberately rounds to the f32 colour component precision.")]
pub(crate) fn color_at(stops: &[(f64, Color)], t: f64) -> Color {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else { return Color::new(0.0, 0.0, 0.0, 0.0) };
    if t <= first.0 {
        return first.1;
    }
    for w in stops.windows(2) {
        if let [(o0, c0), (o1, c1)] = w
            && t <= *o1
        {
            let span = o1 - o0;
            let k = if span > 0.0 { ((t - o0) / span) as f32 } else { 1.0 };
            let mix = |x: f32, y: f32| x + (y - x) * k;
            return Color::new(mix(c0.r, c1.r), mix(c0.g, c1.g), mix(c0.b, c1.b), mix(c0.a, c1.a));
        }
    }
    last.1
}

/// Sweep stops turned by `shift` of a turn: what was at `t` is at `t + shift` (wrapping), with
/// the seam where the gradient starts over kept as a hard step.
pub(crate) fn turn_stops(stops: &[(f64, Color)], shift: f64) -> Vec<(f64, Color)> {
    let d = shift.rem_euclid(1.0);
    if !d.is_finite() || !(1e-9..=1.0 - 1e-9).contains(&d) || stops.is_empty() {
        return stops.to_vec();
    }
    let seam = 1.0 - d;
    let mut out = vec![(0.0, color_at(stops, seam))];
    out.extend(stops.iter().filter(|(o, _)| *o > seam).map(|(o, c)| (o - seam, *c)));
    out.push((d, color_at(stops, 1.0)));
    out.push((d, color_at(stops, 0.0)));
    out.extend(stops.iter().filter(|(o, _)| *o < seam).map(|(o, c)| (o + d, *c)));
    out.push((1.0, color_at(stops, seam)));
    // A stop on the seam itself would repeat at the same offset: keep one of each.
    out.dedup_by(|b, a| (a.0 - b.0).abs() < 1e-9 && a.1 == b.1);
    out
}

/// Gradient `g` on a `w`×`h` layer drawn through `to_view` (local → viewport), its stops faded by
/// `opacity`. `radial` draws an angular or diamond gradient as radial (callers decide when).
pub(crate) fn gradient(gradient: &Gradient, opacity: f32, to_view: Affine, width: f64, height: f64, radial: bool) -> GradOut {
    use figcraft_color::GradientKind;
    let stops: Vec<(f64, Color)> = gradient
        .sorted_stops()
        .iter()
        .map(|stop| (f64::from(stop.position), stop.color.with_alpha(stop.color.a * opacity.clamp(0.0, 1.0))))
        .collect();
    match stops.as_slice() {
        [] => return GradOut::Nothing,
        [(_, c)] => return GradOut::Solid(*c),
        _ => {}
    }
    let transform = to_view * frame(gradient, width, height);
    let [m00, m01, m10, m11, tx, ty] = transform.as_coeffs();
    let determinant = m00 * m11 - m01 * m10;
    let scale = (m00 * m00 + m01 * m01 + m10 * m10 + m11 * m11).max(f64::MIN_POSITIVE);
    if !(determinant.is_finite() && tx.is_finite() && ty.is_finite()) || determinant.abs() <= 1e-12 * scale {
        return GradOut::Nothing;
    }
    let kind = if radial { GradientKind::Radial } else { gradient.kind };
    if kind == GradientKind::Linear {
        let axis = Vec2::new(m00, m01);
        let across = Vec2::new(m10, m11);
        let projection = axis.dot(across) / across.hypot2();
        let normal = axis - across * projection;
        GradOut::Plain(Grad { kind: GradKind::Linear { start: (tx, ty), end: (tx + normal.x, ty + normal.y) }, stops })
    } else {
        let (phi, sx, sy, theta) = svd(m00, m01, m10, m11);
        if !(phi.is_finite() && sx.is_finite() && sy.is_finite() && theta.is_finite()) || sy.abs() <= 1e-9 * sx.abs() {
            return GradOut::Nothing;
        }
        let group = Affine::translate((tx, ty)) * Affine::rotate(phi) * Affine::scale_non_uniform(sx, sy);
        let xf = GroupXf { translate: (tx, ty), rotation: phi.to_degrees(), scale: (sx, sy) };
        let grad = if kind == GradientKind::Angular {
            Grad { kind: GradKind::Sweep { center: (0.0, 0.0) }, stops: turn_stops(&stops, theta / std::f64::consts::TAU) }
        } else {
            Grad { kind: GradKind::Radial { center: (0.0, 0.0), radius: 1.0 }, stops }
        };
        GradOut::Grouped { xf, inverse: group.inverse(), grad }
    }
}

/// `path` (filled even-odd) as a path that fills the same area with the non-zero rule: the faces
/// of its arrangement, each an outer contour with its holes wound the other way. `None` when the
/// path ops can't take it.
pub(crate) fn nonzero(path: &BezPath) -> Option<BezPath> {
    let faces = figcraft_pathops::faces(&[(path.clone(), true)]).ok()?;
    let mut out = BezPath::new();
    for f in faces {
        out.extend(f.path.elements().iter().copied());
    }
    Some(out)
}
