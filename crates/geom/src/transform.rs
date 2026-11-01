//! Decomposing and composing the 2×3 node transforms the design panel shows as X, Y and rotation.

use kurbo::{Affine, Point, Vec2};

/// Rotation in degrees (counter-clockwise on screen, as Figma's rotation field shows it) of `xf`.
pub fn rotation_degrees(xf: Affine) -> f64 {
    let [a, b, ..] = xf.as_coeffs();
    // y-down: positive Figma rotation turns counter-clockwise on screen, i.e. a negative math angle.
    let deg = -b.atan2(a).to_degrees();
    let deg = if deg.abs() < 1e-9 { 0.0 } else { deg };
    if deg <= -180.0 { deg + 360.0 } else { deg }
}

/// The translation part of `xf`.
pub fn translation(xf: Affine) -> Point {
    let c = xf.as_coeffs();
    Point::new(c[4], c[5])
}

/// Replace the translation of `xf`.
pub fn with_translation(xf: Affine, p: Point) -> Affine {
    let c = xf.as_coeffs();
    Affine::new([c[0], c[1], c[2], c[3], p.x, p.y])
}

/// Whether the linear part is a mirror (negative determinant).
pub fn is_flipped(xf: Affine) -> bool {
    xf.determinant() < 0.0
}

/// A transform with the given translation and rotation (degrees, Figma convention).
pub fn from_position_rotation(x: f64, y: f64, degrees: f64) -> Affine {
    Affine::translate(Vec2::new(x, y)) * Affine::rotate(-degrees.to_radians())
}

/// Change the rotation of `xf` to `degrees` about the centre of a `w`×`h` box in its local space,
/// keeping that centre in place and keeping any flip. A flip is kept as a vertical one, the form
/// [`rotation_degrees`] reads (a horizontal flip reads as 180°), so the rotation set reads back.
pub fn set_rotation_about_center(xf: Affine, w: f64, h: f64, degrees: f64) -> Affine {
    let center_local = Point::new(w / 2.0, h / 2.0);
    let center = xf * center_local;
    let flip = if is_flipped(xf) { Affine::FLIP_Y } else { Affine::IDENTITY };
    let rot = Affine::rotate(-degrees.to_radians());
    let lin = rot * flip;
    // Solve translation so that lin * center_local + t = center.
    let mapped = lin * center_local;
    let t = center - mapped;
    let c = lin.as_coeffs();
    Affine::new([c[0], c[1], c[2], c[3], t.x, t.y])
}

/// Rotate `xf` by `degrees` (Figma convention) about the point `pivot` (parent space).
pub fn rotate_about(xf: Affine, pivot: Point, degrees: f64) -> Affine {
    Affine::translate(pivot.to_vec2()) * Affine::rotate(-degrees.to_radians()) * Affine::translate(-pivot.to_vec2()) * xf
}

/// Round a value to `digits` decimals for display (avoids `-0` and float noise).
pub fn round_to(v: f64, digits: i32) -> f64 {
    let f = 10f64.powi(digits);
    let r = (v * f).round() / f;
    if r == 0.0 { 0.0 } else { r }
}
