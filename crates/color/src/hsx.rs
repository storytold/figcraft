//! HSB (HSV) and HSL, two of the models the colour picker offers besides RGB and hex (LCH and
//! OKLCH are in `lch.rs`).

use crate::Color;
use serde::{Deserialize, Serialize};

/// Hue 0..360, saturation and brightness 0..1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hsb {
    pub h: f32,
    pub s: f32,
    pub b: f32,
}

/// Hue 0..360, saturation and lightness 0..1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hsl {
    pub h: f32,
    pub s: f32,
    pub l: f32,
}

fn wrap_hue(h: f32) -> f32 {
    if !h.is_finite() {
        return 0.0;
    }
    let h = h % 360.0;
    if h < 0.0 { h + 360.0 } else { h }
}

fn c01(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

impl Hsb {
    pub fn from_color(c: Color) -> Self {
        let (r, g, b) = (c.r, c.g, c.b);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let h = hue(r, g, b, max, d);
        let s = if max <= 0.0 { 0.0 } else { d / max };
        Self { h, s, b: max }
    }

    pub fn to_color(self) -> Color {
        let (h, s, v) = (wrap_hue(self.h), c01(self.s), c01(self.b));
        let c = v * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = v - c;
        let (r, g, b) = sector(h, c, x);
        Color::new(r + m, g + m, b + m, 1.0)
    }
}

impl Hsl {
    pub fn from_color(c: Color) -> Self {
        let (r, g, b) = (c.r, c.g, c.b);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let l = (max + min) / 2.0;
        let h = hue(r, g, b, max, d);
        let s = if d <= 0.0 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-6) };
        Self { h, s: c01(s), l }
    }

    pub fn to_color(self) -> Color {
        let (h, s, l) = (wrap_hue(self.h), c01(self.s), c01(self.l));
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = l - c / 2.0;
        let (r, g, b) = sector(h, c, x);
        Color::new(r + m, g + m, b + m, 1.0)
    }
}

fn hue(r: f32, g: f32, b: f32, max: f32, d: f32) -> f32 {
    if d <= 0.0 {
        return 0.0;
    }
    let h = if (max - r).abs() < f32::EPSILON {
        60.0 * (((g - b) / d) % 6.0)
    } else if (max - g).abs() < f32::EPSILON {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    wrap_hue(h)
}

fn sector(h: f32, c: f32, x: f32) -> (f32, f32, f32) {
    match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    }
}
