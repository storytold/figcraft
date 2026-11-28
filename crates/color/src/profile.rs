//! Colour profiles: the RGB space a document's colour values live in (sRGB or Display P3), the
//! conversion between them and the ICC profile and PNG `cICP` code points that tag exports.
//!
//! Both spaces share the D65 white point and the sRGB transfer function; they differ only in their
//! primaries. A conversion decodes the transfer function, maps linear RGB through XYZ with the two
//! primaries matrices (derived here from the published chromaticities) and re-encodes. Colours a
//! smaller space can't hold are clipped per channel.

use serde::{Deserialize, Serialize};

use crate::Color;

/// The colour space a document's colour values are in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ColorProfile {
    #[default]
    Srgb,
    DisplayP3,
}

/// A 3×3 matrix, row-major.
pub(crate) type Mat3 = [[f64; 3]; 3];

const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// The D65 white point (CIE 1931 xy) both spaces use.
const D65: (f64, f64) = (0.3127, 0.3290);

impl ColorProfile {
    pub const ALL: [ColorProfile; 2] = [ColorProfile::Srgb, ColorProfile::DisplayP3];

    /// The id used in commands and files (`srgb`, `displayP3`).
    pub fn id(self) -> &'static str {
        match self {
            ColorProfile::Srgb => "srgb",
            ColorProfile::DisplayP3 => "displayP3",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ColorProfile::Srgb => "sRGB",
            ColorProfile::DisplayP3 => "Display P3",
        }
    }

    /// Parse an id or label, ignoring case, spaces, `-` and `_` (`"Display P3"`, `"display-p3"`, `"p3"`).
    pub fn parse(s: &str) -> Option<Self> {
        let k: String = s.chars().filter(|c| !matches!(c, ' ' | '-' | '_')).flat_map(char::to_lowercase).collect();
        match k.as_str() {
            "srgb" => Some(ColorProfile::Srgb),
            "displayp3" | "p3" => Some(ColorProfile::DisplayP3),
            _ => None,
        }
    }

    pub fn is_srgb(&self) -> bool {
        *self == ColorProfile::Srgb
    }

    /// Red, green and blue primaries (CIE 1931 xy).
    fn primaries(self) -> [(f64, f64); 3] {
        match self {
            ColorProfile::Srgb => [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)],
            ColorProfile::DisplayP3 => [(0.680, 0.320), (0.265, 0.690), (0.150, 0.060)],
        }
    }

    /// Linear RGB → CIE XYZ (D65).
    pub(crate) fn to_xyz(self) -> Mat3 {
        rgb_to_xyz(self.primaries(), D65)
    }

    /// The conversion that keeps a colour's appearance when its values move from this space to `to`.
    pub fn conversion_to(self, to: ColorProfile) -> ProfileConversion {
        if self == to {
            return ProfileConversion { m: None };
        }
        let m = mul(&inverse(&to.to_xyz()).unwrap_or(IDENTITY), &self.to_xyz());
        ProfileConversion { m: Some(m) }
    }

    /// PNG `cICP` code points (ITU-T H.273): colour primaries, transfer function (13: the sRGB
    /// curve), matrix coefficients (0: RGB) and full range.
    pub fn cicp(self) -> [u8; 4] {
        match self {
            ColorProfile::Srgb => [1, 13, 0, 1],
            ColorProfile::DisplayP3 => [12, 13, 0, 1],
        }
    }

    /// An ICC v4 matrix/TRC display profile for this space (generated, not copied from anywhere).
    pub fn icc(self) -> Vec<u8> {
        crate::icc::matrix_trc_profile(self.label(), &self.to_xyz(), D65)
    }
}

/// A colour conversion between two profiles (see [`ColorProfile::conversion_to`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileConversion {
    /// Linear RGB → linear RGB; `None` for the identity.
    m: Option<Mat3>,
}

impl ProfileConversion {
    pub fn is_identity(&self) -> bool {
        self.m.is_none()
    }

    /// `c` in the target space (alpha unchanged; out-of-gamut channels clip).
    pub fn apply(&self, c: Color) -> Color {
        let Some(m) = &self.m else { return c };
        let lin = [decode(c.r), decode(c.g), decode(c.b)];
        let [r, g, b] = apply(m, lin);
        Color::new(encode(r), encode(g), encode(b), c.a)
    }

    /// Whether `c` falls outside the target space: a channel clips by more than two 8-bit steps
    /// (the colour picker's gamut warning).
    pub fn clips(&self, c: Color) -> bool {
        let Some(m) = &self.m else { return false };
        apply(m, [decode(c.r), decode(c.g), decode(c.b)]).into_iter().any(outside)
    }

    /// Which premultiplied 8-bit RGBA pixels fall outside the target space (see [`Self::clips`]):
    /// the canvas's soft proof.
    pub fn clipped_pixels(&self, rgba: &[u8]) -> Vec<bool> {
        let pixels = rgba.as_chunks::<4>().0;
        let Some(m) = &self.m else { return vec![false; pixels.len()] };
        let to_linear: [f64; 256] = std::array::from_fn(|i| decode(i as f32 / 255.0));
        let lin = |v: u8| to_linear.get(usize::from(v)).copied().unwrap_or(0.0);
        pixels.iter().map(|&[r, g, b, a]| a != 0 && apply(m, [r, g, b].map(|c| lin(straight(c, a)))).into_iter().any(outside)).collect()
    }

    /// Convert premultiplied 8-bit RGBA pixels in place (table-driven; a no-op for the identity).
    pub fn apply_pixels(&self, rgba: &mut [u8]) {
        let Some(m) = &self.m else { return };
        let to_linear: [f32; 256] = std::array::from_fn(|i| decode(i as f32 / 255.0) as f32);
        let to_encoded: Vec<u8> = (0..ENCODE_STEPS).map(|i| (encode(i as f64 / (ENCODE_STEPS - 1) as f64) * 255.0).round() as u8).collect();
        let m = m.map(|row| row.map(|v| v as f32));
        let lin = |v: u8| to_linear.get(usize::from(v)).copied().unwrap_or(0.0);
        let enc = |l: f32| to_encoded.get((l.clamp(0.0, 1.0) * (ENCODE_STEPS - 1) as f32).round() as usize).copied().unwrap_or(0);
        for [r, g, b, a] in rgba.as_chunks_mut::<4>().0.iter_mut() {
            if *a == 0 {
                continue;
            }
            let alpha = u32::from(*a);
            let v = [lin(straight(*r, *a)), lin(straight(*g, *a)), lin(straight(*b, *a))];
            let out = m.map(|row| enc(row[0] * v[0] + row[1] * v[1] + row[2] * v[2]));
            let premul = |c: u8| if alpha == 255 { c } else { ((u32::from(c) * alpha + 127) / 255) as u8 };
            (*r, *g, *b) = (premul(out[0]), premul(out[1]), premul(out[2]));
        }
    }
}

/// A premultiplied 8-bit channel without its alpha (`a` > 0).
fn straight(c: u8, a: u8) -> u8 {
    let alpha = u32::from(a.max(1));
    if alpha == 255 { c } else { ((u32::from(c) * 255 + alpha / 2) / alpha).min(255) as u8 }
}

/// A linear channel the target space can't hold: below 0 or above 1 by more than two 8-bit steps
/// once encoded (smaller excesses are the rounding of 8-bit values in the other space).
fn outside(l: f64) -> bool {
    const SLACK: f64 = 2.0 / 255.0;
    let enc = |x: f64| if x <= 0.003_130_8 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
    (l < 0.0 && enc(-l) > SLACK) || (l > 1.0 && enc(l) > 1.0 + SLACK)
}

/// Entries of the linear → encoded table [`ProfileConversion::apply_pixels`] uses.
const ENCODE_STEPS: usize = 4096;

/// The sRGB transfer function, encoded value → linear light.
pub(crate) fn decode(v: f32) -> f64 {
    let v = f64::from(if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// The sRGB transfer function, linear light → encoded value (clipped to 0..=1).
pub(crate) fn encode(l: f64) -> f32 {
    let l = if l.is_finite() { l.clamp(0.0, 1.0) } else { 0.0 };
    (if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 }) as f32
}

/// The XYZ of chromaticity `(x, y)` at luminance 1.
pub(crate) fn xy_to_xyz((x, y): (f64, f64)) -> [f64; 3] {
    let y = if y.abs() < 1e-12 { 1e-12 } else { y };
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// The linear RGB → XYZ matrix of an RGB space from its primaries and white point.
fn rgb_to_xyz(primaries: [(f64, f64); 3], white: (f64, f64)) -> Mat3 {
    let [r, g, b] = primaries.map(xy_to_xyz);
    let p = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    // Scale each primary so that RGB (1, 1, 1) lands on the white point.
    let s = apply(&inverse(&p).unwrap_or(IDENTITY), xy_to_xyz(white));
    p.map(|row| [row[0] * s[0], row[1] * s[1], row[2] * s[2]])
}

pub(crate) fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

pub(crate) fn apply(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

pub(crate) fn inverse(m: &Mat3) -> Option<Mat3> {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let co = [e * i - f * h, f * g - d * i, d * h - e * g];
    let det = a * co[0] + b * co[1] + c * co[2];
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    Some([
        [co[0] * inv, (c * h - b * i) * inv, (b * f - c * e) * inv],
        [co[1] * inv, (a * i - c * g) * inv, (c * d - a * f) * inv],
        [co[2] * inv, (b * g - a * h) * inv, (a * e - b * d) * inv],
    ])
}
