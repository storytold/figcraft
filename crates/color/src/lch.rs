//! CIE LCH (CSS `lch()`, Lab with a D50 white) and OKLCH (CSS `oklch()`), the perceptual models
//! the colour picker offers. Colours are read as sRGB, like the hex, RGB, HSL and HSB fields.
//!
//! The matrices are the ones CSS Color 4 publishes (sRGB ↔ XYZ D65, Bradford D65 ↔ D50) and
//! Björn Ottosson's published OKLab matrices.

use crate::Color;
use crate::profile::{apply as mul, decode, encode};
use serde::{Deserialize, Serialize};

/// CIE LCH: lightness 0..100, chroma 0.. (about 0..150 for real colours), hue 0..360.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
}

/// OKLCH: lightness 0..1, chroma 0.. (about 0..0.4 for real colours), hue 0..360.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Oklch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
}

type V3 = [f64; 3];

fn linear_rgb(c: Color) -> V3 {
    [decode(c.r), decode(c.g), decode(c.b)]
}

const SRGB_TO_XYZ: [V3; 3] = [
    [0.412_390_799_265_959_34, 0.357_584_339_383_878, 0.180_480_788_401_834_3],
    [0.212_639_005_871_510_27, 0.715_168_678_767_756, 0.072_192_315_360_733_71],
    [0.019_330_818_715_591_82, 0.119_194_779_794_625_98, 0.950_532_152_249_660_7],
];
const XYZ_TO_SRGB: [V3; 3] = [
    [3.240_969_941_904_522_6, -1.537_383_177_570_094, -0.498_610_760_293_003_4],
    [-0.969_243_636_280_879_6, 1.875_967_501_507_720_2, 0.041_555_057_407_175_59],
    [0.055_630_079_696_993_66, -0.203_976_958_888_976_52, 1.056_971_514_242_878_6],
];
const D65_TO_D50: [V3; 3] = [
    [1.047_929_820_840_548_8, 0.022_946_793_341_019_088, -0.050_192_229_543_135_57],
    [0.029_627_815_688_159_344, 0.990_434_484_573_249, -0.017_073_825_029_385_14],
    [-0.009_243_058_152_591_178, 0.015_055_144_896_577_895, 0.751_874_289_958_000_8],
];
const D50_TO_D65: [V3; 3] = [
    [0.955_473_452_704_218_2, -0.023_098_536_874_261_423, 0.063_259_308_661_021_7],
    [-0.028_369_706_963_208_136, 1.009_995_458_005_822_6, 0.021_041_398_966_943_008],
    [0.012_314_001_688_319_899, -0.020_507_696_433_477_912, 1.330_365_936_608_075_3],
];
const D50_WHITE: V3 = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];
const KAPPA: f64 = 24389.0 / 27.0;
const EPSILON: f64 = 216.0 / 24389.0;

const SRGB_TO_LMS: [V3; 3] = [
    [0.412_221_470_8, 0.536_332_536_3, 0.051_445_992_9],
    [0.211_903_498_2, 0.680_699_545_1, 0.107_396_956_6],
    [0.088_302_461_9, 0.281_718_837_6, 0.629_978_700_5],
];
const LMS_TO_OKLAB: [V3; 3] = [
    [0.210_454_255_3, 0.793_617_785_0, -0.004_072_046_8],
    [1.977_998_495_1, -2.428_592_205_0, 0.450_593_709_9],
    [0.025_904_037_1, 0.782_771_766_2, -0.808_675_766_0],
];
const OKLAB_TO_LMS: [V3; 3] =
    [[1.0, 0.396_337_777_4, 0.215_803_757_3], [1.0, -0.105_561_345_8, -0.063_854_172_8], [1.0, -0.089_484_177_5, -1.291_485_548_0]];
const LMS_TO_SRGB: [V3; 3] = [
    [4.076_741_662_1, -3.307_711_591_3, 0.230_969_929_2],
    [-1.268_438_004_6, 2.609_757_401_1, -0.341_319_396_5],
    [-0.004_196_086_3, -0.703_418_614_7, 1.707_614_701_0],
];

fn lab_from_color(c: Color) -> V3 {
    let xyz = mul(&D65_TO_D50, mul(&SRGB_TO_XYZ, linear_rgb(c)));
    let f = |v: f64| if v > EPSILON { v.cbrt() } else { (KAPPA * v + 16.0) / 116.0 };
    let [fx, fy, fz] = [f(xyz[0] / D50_WHITE[0]), f(xyz[1] / D50_WHITE[1]), f(xyz[2] / D50_WHITE[2])];
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn linear_from_lab([l, a, b]: V3) -> V3 {
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let inv = |f: f64| if f.powi(3) > EPSILON { f.powi(3) } else { (116.0 * f - 16.0) / KAPPA };
    let y = if l > KAPPA * EPSILON { fy.powi(3) } else { l / KAPPA };
    let xyz = [inv(fx) * D50_WHITE[0], y * D50_WHITE[1], inv(fz) * D50_WHITE[2]];
    mul(&XYZ_TO_SRGB, mul(&D50_TO_D65, xyz))
}

fn oklab_from_color(c: Color) -> V3 {
    let lms = mul(&SRGB_TO_LMS, linear_rgb(c));
    mul(&LMS_TO_OKLAB, [lms[0].cbrt(), lms[1].cbrt(), lms[2].cbrt()])
}

fn linear_from_oklab(lab: V3) -> V3 {
    let lms = mul(&OKLAB_TO_LMS, lab);
    mul(&LMS_TO_SRGB, [lms[0].powi(3), lms[1].powi(3), lms[2].powi(3)])
}

/// Lab → (lightness, chroma, hue); grey has hue 0.
fn polar([l, a, b]: V3, grey: f64) -> (f32, f32, f32) {
    let c = a.hypot(b);
    let h = if c < grey { 0.0 } else { b.atan2(a).to_degrees().rem_euclid(360.0) };
    (l as f32, c as f32, h as f32)
}

fn finite(v: f32) -> f64 {
    if v.is_finite() { f64::from(v) } else { 0.0 }
}

fn in_gamut(rgb: V3) -> bool {
    rgb.iter().all(|v| (-1e-6..=1.0 + 1e-6).contains(v))
}

/// The sRGB colour for a lightness, chroma and hue: out of gamut, the chroma comes down (a binary
/// search) until it fits, so lightness and hue stay what was asked for, as CSS gamut mapping does.
fn map_to_srgb(l: f64, c: f64, h: f64, to_linear_rgb: impl Fn(V3) -> V3) -> Color {
    let (sin, cos) = h.to_radians().sin_cos();
    let lab = |c: f64| [l, c * cos, c * sin];
    let mut rgb = to_linear_rgb(lab(c));
    if !in_gamut(rgb) {
        let (mut lo, mut hi) = (0.0, c);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if in_gamut(to_linear_rgb(lab(mid))) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        rgb = to_linear_rgb(lab(lo));
    }
    Color::new(encode(rgb[0]), encode(rgb[1]), encode(rgb[2]), 1.0)
}

impl Lch {
    pub fn from_color(c: Color) -> Self {
        let (l, c, h) = polar(lab_from_color(c), 0.02);
        Self { l, c, h }
    }

    /// Opaque sRGB; out-of-gamut chroma is reduced to the nearest colour sRGB can show.
    pub fn to_color(self) -> Color {
        let l = finite(self.l).clamp(0.0, 100.0);
        let c = finite(self.c).clamp(0.0, 230.0);
        map_to_srgb(l, c, finite(self.h).rem_euclid(360.0), linear_from_lab)
    }
}

impl Oklch {
    pub fn from_color(c: Color) -> Self {
        let (l, c, h) = polar(oklab_from_color(c), 0.000_2);
        Self { l, c, h }
    }

    /// Opaque sRGB; out-of-gamut chroma is reduced to the nearest colour sRGB can show.
    pub fn to_color(self) -> Color {
        let l = finite(self.l).clamp(0.0, 1.0);
        let c = finite(self.c).clamp(0.0, 0.5);
        map_to_srgb(l, c, finite(self.h).rem_euclid(360.0), linear_from_oklab)
    }
}
