//! Procedural shader fills: a built-in pattern worked out per point from a few numbers, not from
//! pixels or a layer.
//!
//! FigCraft's own format: built-in fields ([`ShaderPattern`]) mapped onto a two-colour
//! ramp. It runs no shader source (no GLSL, WGSL or node graph) and reads or writes no other
//! application's shader fills. The field is plain arithmetic on the CPU, the same on every
//! platform and in the browser, so the canvas and the exports draw from one definition.

use serde::{Deserialize, Serialize};

use crate::Color;

const SQRT_3: f64 = 1.732_050_807_568_877_2;

/// The built-in fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ShaderPattern {
    /// Soft clouds: four octaves of value noise.
    #[default]
    Noise,
    /// Parallel bands bent by noise.
    Waves,
    /// Cells around scattered points, shaded by the distance to the nearest point.
    Cells,
    /// Concentric bands around the centre of the filled box.
    Concentric,
}

impl ShaderPattern {
    pub const ALL: [ShaderPattern; 4] = [ShaderPattern::Noise, ShaderPattern::Waves, ShaderPattern::Cells, ShaderPattern::Concentric];

    /// The name in files and command parameters.
    pub fn id(self) -> &'static str {
        match self {
            ShaderPattern::Noise => "noise",
            ShaderPattern::Waves => "waves",
            ShaderPattern::Cells => "cells",
            ShaderPattern::Concentric => "concentric",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ShaderPattern::Noise => "Noise",
            ShaderPattern::Waves => "Waves",
            ShaderPattern::Cells => "Cells",
            ShaderPattern::Concentric => "Concentric",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id().eq_ignore_ascii_case(id.trim()))
    }
}

/// The boundary used to measure a concentric field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ConcentricShape {
    #[default]
    Circle,
    Square,
    Triangle,
    Diamond,
    Hexagon,
}

impl ConcentricShape {
    pub const ALL: [ConcentricShape; 5] =
        [ConcentricShape::Circle, ConcentricShape::Square, ConcentricShape::Triangle, ConcentricShape::Diamond, ConcentricShape::Hexagon];

    pub fn label(self) -> &'static str {
        match self {
            Self::Circle => "Circle",
            Self::Square => "Square",
            Self::Triangle => "Triangle",
            Self::Diamond => "Diamond",
            Self::Hexagon => "Hexagon",
        }
    }
}

/// A procedural shader paint. Values read from a file may be anything: every use goes through
/// [`ShaderPaint::sanitized`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShaderPaint {
    #[serde(default)]
    pub pattern: ShaderPattern,
    /// Boundary of a concentric field.
    #[serde(default)]
    pub shape: ConcentricShape,
    /// Number of alternating colour bands in a concentric field.
    #[serde(default = "default_bands")]
    pub bands: u8,
    /// The colour where the field is 0.
    #[serde(default = "default_a")]
    pub color_a: Color,
    /// The colour where the field is 1.
    #[serde(default = "default_b")]
    pub color_b: Color,
    /// The size of a Noise, Waves or Cells feature in document units.
    #[serde(default = "default_scale")]
    pub scale: f64,
    /// Where the pattern is in its evolution, in seconds. A fixed value: the fill does not play.
    #[serde(default)]
    pub time: f64,
    /// Grain: how far a hash per document unit pushes the field, 0 (none) to 1.
    #[serde(default)]
    pub grain: f32,
    /// Picks one of the patterns that the other values allow.
    #[serde(default)]
    pub seed: u32,
}

fn default_a() -> Color {
    Color::from_rgb8_const(0x1E, 0x1B, 0x4B)
}

fn default_b() -> Color {
    Color::from_rgb8_const(0x7D, 0xD3, 0xFC)
}

fn default_scale() -> f64 {
    ShaderPaint::DEFAULT_SCALE
}

fn default_bands() -> u8 {
    ShaderPaint::DEFAULT_BANDS
}

impl Default for ShaderPaint {
    fn default() -> Self {
        Self::new(ShaderPattern::Noise)
    }
}

impl ShaderPaint {
    /// The smallest and largest feature size, in document units.
    pub const SCALE_RANGE: (f64, f64) = (1.0, 10_000.0);
    pub const DEFAULT_SCALE: f64 = 64.0;
    /// The smallest and largest number of concentric bands.
    pub const BANDS_RANGE: (u8, u8) = (1, 64);
    pub const DEFAULT_BANDS: u8 = 8;
    /// The time wraps into `0..TIME_PERIOD` seconds (a larger one would lose precision).
    pub const TIME_PERIOD: f64 = 3600.0;

    pub fn new(pattern: ShaderPattern) -> Self {
        Self {
            pattern,
            shape: ConcentricShape::default(),
            bands: Self::DEFAULT_BANDS,
            color_a: default_a(),
            color_b: default_b(),
            scale: Self::DEFAULT_SCALE,
            time: 0.0,
            grain: 0.0,
            seed: 0,
        }
    }

    /// Every value in its range: non-finite numbers become the defaults, the scale and grain are
    /// clamped and the time wraps.
    pub fn sanitized(&self) -> Self {
        let (lo, hi) = Self::SCALE_RANGE;
        Self {
            pattern: self.pattern,
            shape: self.shape,
            bands: self.bands.clamp(Self::BANDS_RANGE.0, Self::BANDS_RANGE.1),
            color_a: self.color_a.sanitized(),
            color_b: self.color_b.sanitized(),
            scale: if self.scale.is_finite() { self.scale.clamp(lo, hi) } else { Self::DEFAULT_SCALE },
            time: if self.time.is_finite() { self.time.rem_euclid(Self::TIME_PERIOD) } else { 0.0 },
            grain: if self.grain.is_finite() { self.grain.clamp(0.0, 1.0) } else { 0.0 },
            seed: self.seed,
        }
    }

    /// The field in `0..=1` at a point in the layer's local space. Concentric fields use unit-box
    /// coordinates; use [`ShaderSampler::value_in`] to sample one in a sized box.
    pub fn value(&self, x: f64, y: f64) -> f32 {
        self.sampler().value(x, y)
    }

    /// The colour at a point in the layer's local space.
    pub fn color_at(&self, x: f64, y: f64) -> Color {
        self.sampler().color(x, y)
    }

    /// The sanitised paint ready to sample many points.
    pub fn sampler(&self) -> ShaderSampler {
        let s = self.sanitized();
        ShaderSampler {
            pattern: s.pattern,
            shape: s.shape,
            bands: s.bands,
            a: s.color_a,
            b: s.color_b,
            inv: 1.0 / s.scale,
            time: s.time,
            grain: s.grain,
            seed: s.seed,
        }
    }
}

/// A [`ShaderPaint`] with its values checked once, for sampling point after point.
#[derive(Clone, Copy, Debug)]
pub struct ShaderSampler {
    pattern: ShaderPattern,
    shape: ConcentricShape,
    bands: u8,
    a: Color,
    b: Color,
    inv: f64,
    time: f64,
    grain: f32,
    seed: u32,
}

/// How fast each pattern evolves: field units per second of `time`.
const NOISE_SPEED: f64 = 0.25;
const WAVE_SPEED: f64 = 0.25;
const CELL_SPEED: f64 = 0.5;

impl ShaderSampler {
    /// The field in `0..=1` at `(x, y)`; 0 for a point that isn't finite. Concentric fields
    /// interpret these coordinates in a unit box; use [`Self::value_in`] for a sized box.
    pub fn value(&self, x: f64, y: f64) -> f32 {
        if self.pattern == ShaderPattern::Concentric {
            return self.value_in(x, y, 1.0, 1.0);
        }
        if !(x.is_finite() && y.is_finite()) {
            return 0.0;
        }
        let (u, v) = (x * self.inv, y * self.inv);
        let t = match self.pattern {
            ShaderPattern::Noise => fbm(u, v, self.time * NOISE_SPEED, self.seed, 4),
            ShaderPattern::Waves => {
                let bend = fbm(u * 0.5, v * 0.5, self.time * NOISE_SPEED, self.seed, 2) - 0.5;
                let phase = v + bend * 1.5 - self.time * WAVE_SPEED;
                0.5 + 0.5 * (phase * std::f64::consts::TAU).sin()
            }
            ShaderPattern::Cells => cells(u, v, self.time * CELL_SPEED, self.seed),
            ShaderPattern::Concentric => 0.0,
        };
        let mut t = t as f32;
        if self.grain > 0.0 {
            // One value per document unit, so the grain is the same at every zoom and in exports.
            let h = hash3(cell(x), cell(y), 0x6772_6169, self.seed);
            t += (unit(h) as f32 - 0.5) * self.grain;
        }
        if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 }
    }

    /// The colour at `(x, y)`: the two colours mixed by the field.
    pub fn color(&self, x: f64, y: f64) -> Color {
        self.a.lerp(self.b, self.value(x, y))
    }

    /// The field in a filled box. The coordinates and box dimensions use the same local units,
    /// making the concentric shape and its bands scale with the box instead of the document.
    pub fn value_in(&self, x: f64, y: f64, width: f64, height: f64) -> f32 {
        if self.pattern != ShaderPattern::Concentric {
            return self.value(x, y);
        }
        if !(x.is_finite() && y.is_finite() && width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
            return 0.0;
        }
        let unit = width.max(height);
        let (half_w, half_h) = (width / unit, height / unit);
        let (dx, dy) = ((2.0 * (x / unit) - half_w), (2.0 * (y / unit) - half_h));
        let radius = |x: f64, y: f64| match self.shape {
            ConcentricShape::Circle => x.hypot(y),
            ConcentricShape::Square => x.abs().max(y.abs()),
            ConcentricShape::Triangle => (SQRT_3 * x - y).max(-SQRT_3 * x - y).max(2.0 * y),
            ConcentricShape::Diamond => x.abs() + y.abs(),
            ConcentricShape::Hexagon => (SQRT_3 * x.abs() + y.abs()).max(2.0 * y.abs()),
        };
        let outer =
            [radius(-half_w, -half_h), radius(half_w, -half_h), radius(-half_w, half_h), radius(half_w, half_h)].into_iter().fold(0.0, f64::max);
        let distance = radius(dx, dy);
        let bands = self.bands.clamp(ShaderPaint::BANDS_RANGE.0, ShaderPaint::BANDS_RANGE.1);
        let band = ((distance / outer).clamp(0.0, 1.0) * f64::from(bands)).floor() as u8;
        if band.min(bands.saturating_sub(1)).is_multiple_of(2) { 0.0 } else { 1.0 }
    }

    /// The colour in a filled box.
    pub fn color_in(&self, x: f64, y: f64, width: f64, height: f64) -> Color {
        self.a.lerp(self.b, self.value_in(x, y, width, height))
    }
}

/// The lattice cell a coordinate is in. A float → integer cast saturates, and the lattice wraps
/// every 2³² cells.
fn cell(v: f64) -> u32 {
    v.floor() as i64 as u32
}

/// A hash of a lattice point: multiply and xor-shift rounds, all wrapping.
fn hash3(x: u32, y: u32, z: u32, seed: u32) -> u32 {
    let mut h = seed ^ 0x9E37_79B9;
    for v in [x, y, z] {
        h = (h ^ v).wrapping_mul(0x85EB_CA6B);
        h ^= h >> 15;
        h = h.wrapping_mul(0xC2B2_AE35);
        h ^= h >> 13;
    }
    h ^ (h >> 16)
}

/// A hash as a number in `0..1`.
fn unit(h: u32) -> f64 {
    f64::from(h >> 8) / f64::from(1u32 << 24)
}

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Value noise in `0..1`: hashed lattice values blended smoothly in three dimensions.
fn value_noise(x: f64, y: f64, z: f64, seed: u32) -> f64 {
    let (fx, fy, fz) = (smooth(x - x.floor()), smooth(y - y.floor()), smooth(z - z.floor()));
    let (ix, iy, iz) = (cell(x), cell(y), cell(z));
    let (jx, jy, jz) = (ix.wrapping_add(1), iy.wrapping_add(1), iz.wrapping_add(1));
    let at = |x: u32, y: u32, z: u32| unit(hash3(x, y, z, seed));
    let near = mix(mix(at(ix, iy, iz), at(jx, iy, iz), fx), mix(at(ix, jy, iz), at(jx, jy, iz), fx), fy);
    // A whole `z` (time 0 among them) needs one layer of the lattice only.
    if fz == 0.0 {
        return near;
    }
    let far = mix(mix(at(ix, iy, jz), at(jx, iy, jz), fx), mix(at(ix, jy, jz), at(jx, jy, jz), fx), fy);
    mix(near, far, fz)
}

/// `octaves` of value noise, each twice as fine and half as strong, in `0..1`.
fn fbm(x: f64, y: f64, z: f64, seed: u32, octaves: u32) -> f64 {
    let (mut sum, mut amp, mut total, mut f) = (0.0, 0.5, 0.0, 1.0);
    for o in 0..octaves.min(8) {
        sum += amp * value_noise(x * f, y * f, z * f, seed.wrapping_add(o.wrapping_mul(0x632B_E5AB)));
        total += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    if total > 0.0 { sum / total } else { 0.0 }
}

/// The distance to the nearest of the points scattered one per lattice cell, in `0..=1`. Each
/// point drifts between hashed places as `time` passes.
fn cells(x: f64, y: f64, time: f64, seed: u32) -> f64 {
    let (ix, iy) = (cell(x), cell(y));
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let (step, blend) = (cell(time), smooth(time - time.floor()));
    let place = |cx: u32, cy: u32, step: u32| {
        let h = hash3(cx, cy, step, seed);
        (f64::from(h & 0xFFFF) / 65_536.0, f64::from(h >> 16) / 65_536.0)
    };
    let mut best = f64::MAX;
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            let (cx, cy) = (ix.wrapping_add_signed(dx), iy.wrapping_add_signed(dy));
            let (mut px, mut py) = place(cx, cy, step);
            if blend > 0.0 {
                let (qx, qy) = place(cx, cy, step.wrapping_add(1));
                (px, py) = (mix(px, qx, blend), mix(py, qy, blend));
            }
            let (ex, ey) = (f64::from(dx) + px - fx, f64::from(dy) + py - fy);
            best = best.min(ex * ex + ey * ey);
        }
    }
    best.sqrt().clamp(0.0, 1.0)
}
