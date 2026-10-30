//! Node property types: strokes, effects, corners, constraints, auto layout, grids, exports.

use figcraft_color::{BlendMode, Color};
use serde::{Deserialize, Serialize};

/// Where a stroke sits relative to the shape edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StrokeAlign {
    #[default]
    Inside,
    Center,
    Outside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StrokeCap {
    #[default]
    None,
    Round,
    Square,
    LineArrow,
    TriangleArrow,
    ReversedTriangle,
    CircleFilled,
    DiamondFilled,
}

impl StrokeCap {
    pub const ALL: [StrokeCap; 8] = [
        StrokeCap::None,
        StrokeCap::Round,
        StrokeCap::Square,
        StrokeCap::LineArrow,
        StrokeCap::TriangleArrow,
        StrokeCap::ReversedTriangle,
        StrokeCap::CircleFilled,
        StrokeCap::DiamondFilled,
    ];
    pub fn label(self) -> &'static str {
        match self {
            StrokeCap::None => "None",
            StrokeCap::Round => "Round",
            StrokeCap::Square => "Square",
            StrokeCap::LineArrow => "Line arrow",
            StrokeCap::TriangleArrow => "Triangle arrow",
            StrokeCap::ReversedTriangle => "Reversed triangle",
            StrokeCap::CircleFilled => "Circle",
            StrokeCap::DiamondFilled => "Diamond",
        }
    }
    pub fn is_arrow(self) -> bool {
        !matches!(self, StrokeCap::None | StrokeCap::Round | StrokeCap::Square)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StrokeJoin {
    #[default]
    Miter,
    Bevel,
    Round,
}

impl StrokeJoin {
    pub const ALL: [StrokeJoin; 3] = [StrokeJoin::Miter, StrokeJoin::Bevel, StrokeJoin::Round];
}

/// Stroke geometry shared by every stroke paint of a node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StrokeProps {
    pub weight: f64,
    pub align: StrokeAlign,
    /// Per-side weights `[top, right, bottom, left]` (rectangles and frames only).
    pub sides: Option<[f64; 4]>,
    pub cap_start: StrokeCap,
    pub cap_end: StrokeCap,
    pub join: StrokeJoin,
    pub miter_limit: f64,
    /// Dash pattern (dash, gap, …); empty = solid.
    pub dashes: Vec<f64>,
    /// Variable width: how the width changes along the path (`None` = uniform).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_profile: Option<WidthProfile>,
    /// A brush the stroke is drawn with instead of a plain band (`None`: plain).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brush: Option<Brush>,
}

/// A brush stroke: the stroke drawn with a built-in brush shape (`preset`, an id of
/// `figcraft_pathops::brush::PRESETS`), stretched along the path or scattered along it every
/// `spacing` stroke weights with repeatable jitter from `seed`. A width profile scales it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Brush {
    pub preset: String,
    /// Scatter: distance between copies, in stroke weights.
    pub spacing: f64,
    /// Scatter: how much copies vary in size, 0..1.
    pub size_jitter: f64,
    /// Scatter: how far copies turn from the path's direction, 0..1 (1: up to half a turn).
    pub rotation_jitter: f64,
    /// Scatter: how far copies stray from the path, in stroke weights, 0..4.
    pub scatter: f64,
    pub seed: u32,
}

impl Default for Brush {
    fn default() -> Self {
        Self { preset: String::new(), spacing: 1.5, size_jitter: 0.0, rotation_jitter: 0.0, scatter: 0.0, seed: 0 }
    }
}

impl Brush {
    /// A brush of `preset` with default settings.
    pub fn new(preset: impl Into<String>) -> Self {
        Self { preset: preset.into(), ..Self::default() }
    }

    /// Every value in range (NaN: the default).
    pub fn sanitized(&self) -> Self {
        let c = |v: f64, lo: f64, hi: f64, d: f64| if v.is_finite() { v.clamp(lo, hi) } else { d };
        Self {
            preset: self.preset.clone(),
            spacing: c(self.spacing, 0.1, 100.0, 1.5),
            size_jitter: c(self.size_jitter, 0.0, 1.0, 0.0),
            rotation_jitter: c(self.rotation_jitter, 0.0, 1.0, 0.0),
            scatter: c(self.scatter, 0.0, 4.0, 0.0),
            seed: self.seed,
        }
    }

    /// How much wider than the stroke weight the brush can paint (larger copies, strays).
    pub fn reach_factor(&self) -> f64 {
        let b = self.sanitized();
        1.0 + b.size_jitter + 2.0 * b.scatter
    }
}

impl Default for StrokeProps {
    fn default() -> Self {
        Self {
            weight: 1.0,
            align: StrokeAlign::Inside,
            sides: None,
            cap_start: StrokeCap::None,
            cap_end: StrokeCap::None,
            join: StrokeJoin::Miter,
            miter_limit: 4.0,
            dashes: Vec::new(),
            width_profile: None,
            brush: None,
        }
    }
}

impl StrokeProps {
    /// The variable width profile, when there is one with points.
    pub fn profile(&self) -> Option<&WidthProfile> {
        self.width_profile.as_ref().filter(|p| !p.points.is_empty())
    }

    /// The widest the stroke gets anywhere along the path (the weight, scaled by a width profile
    /// and widened by a brush's reach).
    pub fn max_weight(&self) -> f64 {
        self.profile().map_or(self.weight, |p| self.weight * p.max_scale()) * self.brush.as_ref().map_or(1.0, Brush::reach_factor)
    }

    /// The miter limit as the angle (degrees) below which joins bevel: limit = 1 / sin(angle / 2),
    /// so the default limit 4 is ~28.96°.
    pub fn miter_angle(&self) -> f64 {
        2.0 * (1.0 / self.miter_limit.clamp(1.0, 100.0)).asin().to_degrees()
    }

    /// The miter limit for a miter angle in degrees (kept within the limit's 1–100 range).
    pub fn miter_limit_for(angle: f64) -> f64 {
        let a = if angle.is_finite() { angle.clamp(1.0, 180.0) } else { 28.955 };
        (1.0 / (a / 2.0).to_radians().sin()).clamp(1.0, 100.0)
    }
}

/// One point of a variable width profile.
///
/// The width can differ on the two sides of the path. Looking along the path on screen (y down),
/// the left side reaches `weight × scale × (1 − bias) / 2` from the centre line and the right side
/// `weight × scale × (1 + bias) / 2`. A side's **side scale** (`scale × (1 ∓ bias)`) is in the units
/// of `scale`: a symmetric point has both side scales equal to `scale`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WidthPoint {
    /// Position along the path by length: 0 at its start, 1 at its end (each subpath on its own).
    pub t: f64,
    /// The stroke's width here as a multiple of the stroke weight (the mean of the side scales).
    pub scale: f64,
    /// How much wider the right side is than the left, -1..=1 (0: symmetric; 1: all on the right).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub bias: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// A side of a path for one-sided widths (looking along the path on screen, y down).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WidthSide {
    Left,
    Right,
}

impl WidthSide {
    pub fn from_id(s: &str) -> Option<WidthSide> {
        match s.to_ascii_lowercase().as_str() {
            "left" => Some(WidthSide::Left),
            "right" => Some(WidthSide::Right),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            WidthSide::Left => "left",
            WidthSide::Right => "right",
        }
    }
}

impl WidthPoint {
    /// A symmetric point.
    pub const fn new(t: f64, scale: f64) -> Self {
        Self { t, scale, bias: 0.0 }
    }

    /// A point from its left and right side scales (negative or non-finite sides count as 0).
    pub fn from_sides(t: f64, left: f64, right: f64) -> Self {
        let clean = |v: f64| if v.is_finite() { v.max(0.0) } else { 0.0 };
        let (l, r) = (clean(left), clean(right));
        let sum = l + r;
        let bias = if sum > 1e-12 { ((r - l) / sum).clamp(-1.0, 1.0) } else { 0.0 };
        Self { t, scale: sum / 2.0, bias }
    }

    /// The left and right side scales.
    pub fn sides(&self) -> (f64, f64) {
        (self.scale * (1.0 - self.bias), self.scale * (1.0 + self.bias))
    }

    /// Whether both sides are as wide.
    pub fn symmetric(&self) -> bool {
        self.bias == 0.0
    }
}

/// A variable width profile: stroke widths at positions along the path, interpolated smoothly
/// (monotone cubic, so the width never overshoots its neighbouring points) and held flat before
/// the first and after the last point.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WidthProfile {
    /// Sorted by `t` (see [`WidthProfile::sanitized`]).
    pub points: Vec<WidthPoint>,
}

impl WidthProfile {
    /// The most points a profile keeps.
    pub const MAX_POINTS: usize = 64;
    /// The widest a point can be, as a multiple of the stroke weight.
    pub const MAX_SCALE: f64 = 1000.0;

    /// A profile from `(t, scale)` pairs (sanitized).
    pub fn new(points: impl IntoIterator<Item = (f64, f64)>) -> Self {
        Self { points: points.into_iter().map(|(t, s)| WidthPoint::new(t, s)).collect() }.sanitized()
    }

    /// Uniform width with a point at each end (where the width tool starts from).
    pub fn uniform() -> Self {
        Self::new([(0.0, 1.0), (1.0, 1.0)])
    }

    /// Finite points only, `t` in 0..=1, scales in 0..=[`Self::MAX_SCALE`], sorted by `t`, at
    /// most [`Self::MAX_POINTS`].
    pub fn sanitized(&self) -> Self {
        let mut points: Vec<WidthPoint> = self
            .points
            .iter()
            .filter(|p| p.t.is_finite() && p.scale.is_finite())
            .take(Self::MAX_POINTS)
            .map(|p| WidthPoint {
                t: p.t.clamp(0.0, 1.0),
                scale: p.scale.clamp(0.0, Self::MAX_SCALE),
                bias: if p.bias.is_finite() { p.bias.clamp(-1.0, 1.0) } else { 0.0 },
            })
            .collect();
        points.sort_by(|a, b| a.t.total_cmp(&b.t));
        Self { points }
    }

    /// The width scale at `t` (0..1 along the path): the mean of the two sides' scales.
    pub fn scale_at(&self, t: f64) -> f64 {
        let (l, r) = self.sides_at(t);
        (l + r) / 2.0
    }

    /// The left and right side scales at `t`, each side interpolated on its own.
    pub fn sides_at(&self, t: f64) -> (f64, f64) {
        if self.symmetric() {
            let s = self.interp(t, |p| p.scale);
            return (s, s);
        }
        (self.interp(t, |p| p.sides().0), self.interp(t, |p| p.sides().1))
    }

    /// Whether every point is as wide on both sides.
    pub fn symmetric(&self) -> bool {
        self.points.iter().all(WidthPoint::symmetric)
    }

    /// `value` of the points at `t`: monotone cubic between points, held flat before the first and
    /// after the last.
    fn interp(&self, t: f64, value: impl Fn(&WidthPoint) -> f64) -> f64 {
        let pts = &self.points;
        let (Some(first), Some(last)) = (pts.first(), pts.last()) else { return 1.0 };
        if !t.is_finite() || t <= first.t {
            return value(first);
        }
        if t >= last.t {
            return value(last);
        }
        let k = pts.partition_point(|p| p.t <= t).saturating_sub(1);
        let (Some(a), Some(b)) = (pts.get(k), pts.get(k + 1)) else { return value(last) };
        let h = b.t - a.t;
        if h <= 1e-12 {
            return value(b);
        }
        let u = (t - a.t) / h;
        let (u2, u3) = (u * u, u * u * u);
        let w = (2.0 * u3 - 3.0 * u2 + 1.0) * value(a)
            + (u3 - 2.0 * u2 + u) * h * self.tangent(k, &value)
            + (3.0 * u2 - 2.0 * u3) * value(b)
            + (u3 - u2) * h * self.tangent(k + 1, &value);
        w.max(0.0)
    }

    /// Slope of `value` from point `i` to `i + 1` (0 when they share a position).
    fn secant(&self, i: usize, value: &impl Fn(&WidthPoint) -> f64) -> f64 {
        match (self.points.get(i), self.points.get(i + 1)) {
            (Some(a), Some(b)) if b.t - a.t > 1e-12 => (value(b) - value(a)) / (b.t - a.t),
            _ => 0.0,
        }
    }

    /// Fritsch–Carlson style tangent at point `i`: one-sided at the ends, the harmonic mean of the
    /// neighbouring slopes inside, flat at local extremes (keeps the curve monotone between points).
    fn tangent(&self, i: usize, value: &impl Fn(&WidthPoint) -> f64) -> f64 {
        let last = self.points.len().saturating_sub(1);
        if i == 0 {
            return self.secant(0, value);
        }
        if i >= last {
            return self.secant(last.saturating_sub(1), value);
        }
        let (d0, d1) = (self.secant(i - 1, value), self.secant(i, value));
        if d0 * d1 <= 0.0 { 0.0 } else { 2.0 * d0 * d1 / (d0 + d1) }
    }

    /// The largest side scale (the interpolation never exceeds its points): the widest the
    /// stroke reaches from its centre line is `weight × max_scale / 2`.
    pub fn max_scale(&self) -> f64 {
        self.points.iter().map(|p| p.sides().0.max(p.sides().1)).fold(0.0, f64::max)
    }

    /// Add a symmetric point (kept sorted); returns its index.
    pub fn insert(&mut self, t: f64, scale: f64) -> usize {
        self.insert_point(WidthPoint::new(t, if scale.is_finite() { scale } else { 1.0 }))
    }

    /// Add a point at `t` as wide on each side as the stroke already is there; returns its index.
    pub fn insert_here(&mut self, t: f64) -> usize {
        let (l, r) = self.sides_at(t);
        self.insert_point(WidthPoint::from_sides(t, l, r))
    }

    fn insert_point(&mut self, mut p: WidthPoint) -> usize {
        p.t = if p.t.is_finite() { p.t.clamp(0.0, 1.0) } else { 0.0 };
        p.scale = p.scale.clamp(0.0, Self::MAX_SCALE);
        let i = self.points.partition_point(|q| q.t <= p.t);
        self.points.insert(i, p);
        i
    }

    /// Set one side of point `i` to side scale `scale`, keeping the other side. Returns false when
    /// there is no such point.
    pub fn set_side(&mut self, i: usize, side: WidthSide, scale: f64) -> bool {
        let Some(p) = self.points.get_mut(i) else { return false };
        if !scale.is_finite() {
            return true;
        }
        let s = scale.clamp(0.0, 2.0 * Self::MAX_SCALE);
        let (l, r) = p.sides();
        let q = match side {
            WidthSide::Left => WidthPoint::from_sides(p.t, s, r),
            WidthSide::Right => WidthPoint::from_sides(p.t, l, s),
        };
        p.scale = q.scale.min(Self::MAX_SCALE);
        p.bias = q.bias;
        true
    }

    /// Move point `i` along the path (kept between its neighbours, so indices stay put) and/or
    /// change its scale. Returns false when there is no such point.
    pub fn set_point(&mut self, i: usize, t: Option<f64>, scale: Option<f64>) -> bool {
        let lo = i.checked_sub(1).and_then(|j| self.points.get(j)).map_or(0.0, |p| p.t);
        let hi = self.points.get(i + 1).map_or(1.0, |p| p.t);
        let Some(p) = self.points.get_mut(i) else { return false };
        if let Some(t) = t.filter(|t| t.is_finite()) {
            p.t = t.clamp(lo, hi.max(lo));
        }
        // A new width makes the point symmetric again.
        if let Some(s) = scale.filter(|s| s.is_finite()) {
            p.scale = s.clamp(0.0, Self::MAX_SCALE);
            p.bias = 0.0;
        }
        true
    }

    /// The preset this profile is, if any.
    pub fn preset(&self) -> Option<WidthPreset> {
        WidthPreset::ALL.into_iter().find(|p| p.profile().as_ref() == Some(self))
    }
}

/// The width profiles offered in the stroke settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidthPreset {
    Uniform,
    /// Full width at the start, a point at the end.
    Wedge,
    /// Pointed at both ends, full width in between.
    Taper,
    /// Widest in the middle, pointed at both ends.
    Eye,
    /// Full width with rounded-off ends.
    Rounded,
    /// Thin in the middle, full width at the ends.
    Pinch,
}

impl WidthPreset {
    pub const ALL: [WidthPreset; 6] =
        [WidthPreset::Uniform, WidthPreset::Wedge, WidthPreset::Taper, WidthPreset::Eye, WidthPreset::Rounded, WidthPreset::Pinch];

    pub fn id(self) -> &'static str {
        match self {
            WidthPreset::Uniform => "uniform",
            WidthPreset::Wedge => "wedge",
            WidthPreset::Taper => "taper",
            WidthPreset::Eye => "eye",
            WidthPreset::Rounded => "rounded",
            WidthPreset::Pinch => "pinch",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WidthPreset::Uniform => "Uniform",
            WidthPreset::Wedge => "Wedge",
            WidthPreset::Taper => "Taper both ends",
            WidthPreset::Eye => "Eye",
            WidthPreset::Rounded => "Rounded",
            WidthPreset::Pinch => "Pinch",
        }
    }

    pub fn from_id(s: &str) -> Option<WidthPreset> {
        let s: String = s.chars().filter(char::is_ascii_alphanumeric).collect();
        WidthPreset::ALL
            .into_iter()
            .find(|p| p.id().eq_ignore_ascii_case(&s) || p.label().replace(' ', "").eq_ignore_ascii_case(&s))
            .or_else(|| s.eq_ignore_ascii_case("bulge").then_some(WidthPreset::Eye))
    }

    /// The preset's profile (`None` = uniform).
    pub fn profile(self) -> Option<WidthProfile> {
        let pts: &[(f64, f64)] = match self {
            WidthPreset::Uniform => return None,
            WidthPreset::Wedge => &[(0.0, 1.0), (1.0, 0.0)],
            WidthPreset::Taper => &[(0.0, 0.0), (0.2, 1.0), (0.8, 1.0), (1.0, 0.0)],
            WidthPreset::Eye => &[(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)],
            WidthPreset::Rounded => &[(0.0, 0.3), (0.05, 0.8), (0.15, 1.0), (0.85, 1.0), (0.95, 0.8), (1.0, 0.3)],
            WidthPreset::Pinch => &[(0.0, 1.0), (0.5, 0.25), (1.0, 1.0)],
        };
        Some(WidthProfile::new(pts.iter().copied()))
    }
}

/// A shadow (drop or inner).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shadow {
    pub color: Color,
    pub offset: (f64, f64),
    /// Blur radius.
    pub radius: f64,
    pub spread: f64,
    pub blend: BlendMode,
    /// Drop shadows only: show behind transparent areas of the layer.
    #[serde(alias = "showBehind")]
    pub show_behind: bool,
}

impl Default for Shadow {
    fn default() -> Self {
        Self { color: Color::new(0.0, 0.0, 0.0, 0.25), offset: (0.0, 4.0), radius: 4.0, spread: 0.0, blend: BlendMode::Normal, show_behind: false }
    }
}

/// How shadow spread offsets a sharp corner: the outline's edges move out (or in) in parallel and
/// meet in a mitre up to this many times the spread from the corner, then bevel.
pub const SPREAD_MITER_LIMIT: f64 = 4.0;

/// How many colours noise grains take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum NoiseMode {
    /// One colour.
    #[default]
    Mono,
    /// Two colours (`color` and `secondary`).
    Duo,
    /// Grains of every colour, at `opacity`.
    Multi,
}

impl NoiseMode {
    pub const ALL: [NoiseMode; 3] = [NoiseMode::Mono, NoiseMode::Duo, NoiseMode::Multi];
    pub fn label(self) -> &'static str {
        match self {
            NoiseMode::Mono => "Mono",
            NoiseMode::Duo => "Duo",
            NoiseMode::Multi => "Multi",
        }
    }
}

/// A grain size (document units) on both axes, sanitised.
fn grain_of(size: f64, size_y: Option<f64>) -> (f64, f64) {
    let ok = |v: f64| if v.is_finite() { v.clamp(0.01, 1000.0) } else { 0.5 };
    (ok(size), ok(size_y.unwrap_or(size)))
}

/// Grain over the layer's pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Noise {
    pub mode: NoiseMode,
    /// Grain size (document units) across; also down unless `size_y` is set.
    pub size: f64,
    #[serde(alias = "sizeY", skip_serializing_if = "Option::is_none")]
    pub size_y: Option<f64>,
    /// Share of grains drawn (0–1).
    pub density: f64,
    /// Mono and Duo: the grains' colour and opacity.
    pub color: Color,
    /// Duo: the other grains' colour.
    pub secondary: Color,
    /// Multi: the grains' opacity (0–1).
    pub opacity: f32,
    pub blend: BlendMode,
}

impl Default for Noise {
    fn default() -> Self {
        Self {
            mode: NoiseMode::Mono,
            size: 0.5,
            size_y: None,
            density: 0.5,
            color: Color::new(0.0, 0.0, 0.0, 0.25),
            secondary: Color::new(1.0, 1.0, 1.0, 0.25),
            opacity: 0.25,
            blend: BlendMode::Normal,
        }
    }
}

impl Noise {
    /// Grain size across and down (document units, sanitised).
    pub fn grain(&self) -> (f64, f64) {
        grain_of(self.size, self.size_y)
    }
}

/// Roughened edges: the layer's pixels scattered by up to `radius`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Texture {
    /// Grain size (document units) across; also down unless `size_y` is set.
    pub size: f64,
    #[serde(alias = "sizeY", skip_serializing_if = "Option::is_none")]
    pub size_y: Option<f64>,
    /// How far pixels scatter (document units): past the layer's edge unless clipped.
    pub radius: f64,
    /// Keep the texture inside the layer's own pixels.
    #[serde(alias = "clipToShape")]
    pub clip_to_shape: bool,
}

impl Default for Texture {
    fn default() -> Self {
        Self { size: 0.5, size_y: None, radius: 4.0, clip_to_shape: false }
    }
}

impl Texture {
    /// Grain size across and down (document units, sanitised).
    pub fn grain(&self) -> (f64, f64) {
        grain_of(self.size, self.size_y)
    }
}

/// Glass: the backdrop seen through a pane whose curved edges bend it, split its colours and catch
/// the light, optionally frosted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Glass {
    /// Where the light comes from, in degrees clockwise from the top.
    #[serde(alias = "lightAngle")]
    pub light_angle: f64,
    /// Brightness of the edge highlights (0–1).
    #[serde(alias = "lightIntensity")]
    pub light_intensity: f64,
    /// How strongly the curved edge bends the backdrop (0–1).
    pub refraction: f64,
    /// How far the curved edge reaches in from the border (document units, at least 1).
    pub depth: f64,
    /// Colour splitting along the curved edge (0–1).
    pub dispersion: f64,
    /// Blur of the backdrop (a blur radius, document units).
    pub frost: f64,
    /// How widely the light spreads over the glass (0–1).
    pub splay: f64,
}

impl Default for Glass {
    fn default() -> Self {
        Self { light_angle: -45.0, light_intensity: 0.8, refraction: 0.8, depth: 20.0, dispersion: 0.5, frost: 4.0, splay: 0.0 }
    }
}

impl Glass {
    /// The values clamped to their ranges (non-finite ones at their defaults).
    pub fn sanitized(&self) -> Glass {
        let d = Glass::default();
        let ok = |v: f64, def: f64, lo: f64, hi: f64| if v.is_finite() { v.clamp(lo, hi) } else { def };
        Glass {
            light_angle: ok(self.light_angle, d.light_angle, -3600.0, 3600.0),
            light_intensity: ok(self.light_intensity, d.light_intensity, 0.0, 1.0),
            refraction: ok(self.refraction, d.refraction, 0.0, 1.0),
            depth: ok(self.depth, d.depth, 1.0, 1000.0),
            dispersion: ok(self.dispersion, d.dispersion, 0.0, 1.0),
            frost: ok(self.frost, d.frost, 0.0, 1000.0),
            splay: ok(self.splay, d.splay, 0.0, 1.0),
        }
    }

    /// Furthest the curved edge moves the backdrop (document units).
    pub fn max_bend(&self) -> f64 {
        let g = self.sanitized();
        g.refraction * g.depth * (1.0 + g.dispersion * 0.25)
    }
}

/// How a blur's radius varies across the layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum BlurType {
    /// The same radius everywhere.
    #[default]
    #[serde(alias = "normal", alias = "NORMAL", alias = "UNIFORM")]
    Uniform,
    /// The radius ramps from `start_radius` at `start` to `radius` at `end`.
    #[serde(alias = "PROGRESSIVE")]
    Progressive,
}

impl BlurType {
    pub const ALL: [BlurType; 2] = [BlurType::Uniform, BlurType::Progressive];
    pub fn label(self) -> &'static str {
        match self {
            BlurType::Uniform => "Uniform",
            BlurType::Progressive => "Progressive",
        }
    }
}

/// The largest blur radius drawn (document units); larger ones look the same anyway.
pub const MAX_BLUR_RADIUS: f64 = 10_000.0;

/// A layer or background blur: uniform, or progressive (the radius ramps linearly along a line
/// between two points in the layer's unit box, like a linear gradient's handles).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Blur {
    /// The blur radius (progressive: at `end` and past it).
    pub radius: f64,
    #[serde(alias = "blurType")]
    pub blur_type: BlurType,
    /// Progressive: the radius at `start` and before it.
    #[serde(alias = "startRadius")]
    pub start_radius: f64,
    /// Progressive: where the ramp starts, in the layer's unit box (0..1 across and down).
    #[serde(alias = "startOffset")]
    pub start: (f64, f64),
    /// Progressive: where the ramp ends, in the layer's unit box.
    #[serde(alias = "endOffset")]
    pub end: (f64, f64),
}

impl Default for Blur {
    fn default() -> Self {
        Self { radius: 4.0, blur_type: BlurType::Uniform, start_radius: 0.0, start: (0.5, 0.0), end: (0.5, 1.0) }
    }
}

impl Blur {
    /// A uniform blur of `radius`.
    pub fn uniform(radius: f64) -> Self {
        Self { radius, ..Self::default() }
    }

    pub fn is_progressive(&self) -> bool {
        self.blur_type == BlurType::Progressive
    }

    /// The values made safe to draw: radii finite in 0..[`MAX_BLUR_RADIUS`], points finite
    /// (non-finite ones at their defaults) within a thousand boxes of the layer.
    pub fn sanitized(&self) -> Blur {
        let d = Blur::default();
        let radius = |v: f64| if v.is_finite() { v.clamp(0.0, MAX_BLUR_RADIUS) } else { 0.0 };
        let point = |p: (f64, f64), def: (f64, f64)| {
            let ok = |v: f64, dv: f64| if v.is_finite() { v.clamp(-1000.0, 1000.0) } else { dv };
            (ok(p.0, def.0), ok(p.1, def.1))
        };
        Blur {
            radius: radius(self.radius),
            blur_type: self.blur_type,
            start_radius: radius(self.start_radius),
            start: point(self.start, d.start),
            end: point(self.end, d.end),
        }
    }

    /// The largest radius anywhere on the layer (sanitised).
    pub fn max_radius(&self) -> f64 {
        let b = self.sanitized();
        if b.is_progressive() { b.radius.max(b.start_radius) } else { b.radius }
    }

    /// The same blur scaled by `k` (radii; the unit-box points stay).
    pub fn scaled(&self, k: f64) -> Blur {
        Blur { radius: self.radius * k, start_radius: self.start_radius * k, ..self.clone() }
    }
}

/// One effect in a node's effect list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EffectKind {
    DropShadow(Shadow),
    InnerShadow(Shadow),
    LayerBlur(Blur),
    BackgroundBlur(Blur),
    /// Grain over the layer.
    Noise(Noise),
    /// Roughened edges.
    Texture(Texture),
    /// The backdrop through curved glass.
    Glass(Glass),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    #[serde(flatten)]
    pub kind: EffectKind,
    #[serde(default = "yes")]
    pub visible: bool,
}

fn yes() -> bool {
    true
}

impl Effect {
    /// Effect types in the order the Design panel lists them: (label, type name).
    pub const TYPES: [(&str, &str); 7] = [
        ("Drop shadow", "dropShadow"),
        ("Inner shadow", "innerShadow"),
        ("Layer blur", "layerBlur"),
        ("Background blur", "backgroundBlur"),
        ("Noise", "noise"),
        ("Texture", "texture"),
        ("Glass", "glass"),
    ];

    fn of(kind: EffectKind) -> Self {
        Self { kind, visible: true }
    }
    pub fn drop_shadow() -> Self {
        Self::of(EffectKind::DropShadow(Shadow::default()))
    }
    pub fn inner_shadow() -> Self {
        Self::of(EffectKind::InnerShadow(Shadow { offset: (0.0, 4.0), ..Shadow::default() }))
    }
    pub fn layer_blur() -> Self {
        Self::of(EffectKind::LayerBlur(Blur::default()))
    }
    pub fn background_blur() -> Self {
        Self::of(EffectKind::BackgroundBlur(Blur::default()))
    }
    pub fn noise() -> Self {
        Self::of(EffectKind::Noise(Noise::default()))
    }
    pub fn texture() -> Self {
        Self::of(EffectKind::Texture(Texture::default()))
    }
    pub fn glass() -> Self {
        Self::of(EffectKind::Glass(Glass::default()))
    }

    /// A new effect of a type named like `dropShadow`, `drop shadow` or `DROP_SHADOW` (`blur` is
    /// a layer blur), with its defaults.
    pub fn of_type(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().replace(['_', ' ', '-'], "").as_str() {
            "dropshadow" => Self::drop_shadow(),
            "innershadow" => Self::inner_shadow(),
            "layerblur" | "blur" => Self::layer_blur(),
            "backgroundblur" => Self::background_blur(),
            "noise" => Self::noise(),
            "texture" => Self::texture(),
            "glass" => Self::glass(),
            _ => return None,
        })
    }

    pub fn label(&self) -> &'static str {
        match self.kind {
            EffectKind::DropShadow(_) => "Drop shadow",
            EffectKind::InnerShadow(_) => "Inner shadow",
            EffectKind::LayerBlur(_) => "Layer blur",
            EffectKind::BackgroundBlur(_) => "Background blur",
            EffectKind::Noise(_) => "Noise",
            EffectKind::Texture(_) => "Texture",
            EffectKind::Glass(_) => "Glass",
        }
    }
    /// How far outside the node's bounds this effect can paint (a shadow's spread at a sharp
    /// corner can reach further: see [`Node::paint_reach`](crate::Node::paint_reach)).
    pub fn reach(&self) -> f64 {
        if !self.visible {
            return 0.0;
        }
        match &self.kind {
            EffectKind::DropShadow(s) => s.radius.abs() * 1.5 + s.spread.max(0.0) + s.offset.0.abs().max(s.offset.1.abs()),
            EffectKind::LayerBlur(b) => b.max_radius() * 1.5,
            EffectKind::Texture(t) if !t.clip_to_shape => t.radius.abs(),
            _ => 0.0,
        }
    }
    /// Whether this is a visible layer blur or texture: what moves the layer's own pixels, one
    /// after another in list order.
    pub fn moves_the_layer(&self) -> bool {
        self.visible && matches!(self.kind, EffectKind::LayerBlur(_) | EffectKind::Texture(_))
    }
    /// A layer or background blur's settings.
    pub fn blur(&self) -> Option<&Blur> {
        match &self.kind {
            EffectKind::LayerBlur(b) | EffectKind::BackgroundBlur(b) => Some(b),
            _ => None,
        }
    }
    /// Mutable access to [`Effect::blur`].
    pub fn blur_mut(&mut self) -> Option<&mut Blur> {
        match &mut self.kind {
            EffectKind::LayerBlur(b) | EffectKind::BackgroundBlur(b) => Some(b),
            _ => None,
        }
    }
    /// The effect's colour (`secondary`: a duo noise's second colour), if it has one.
    pub fn color(&self, secondary: bool) -> Option<Color> {
        match (&self.kind, secondary) {
            (EffectKind::DropShadow(s) | EffectKind::InnerShadow(s), false) => Some(s.color),
            (EffectKind::Noise(n), false) => Some(n.color),
            (EffectKind::Noise(n), true) => Some(n.secondary),
            _ => None,
        }
    }
    /// Mutable access to [`Effect::color`].
    pub fn color_mut(&mut self, secondary: bool) -> Option<&mut Color> {
        match (&mut self.kind, secondary) {
            (EffectKind::DropShadow(s) | EffectKind::InnerShadow(s), false) => Some(&mut s.color),
            (EffectKind::Noise(n), false) => Some(&mut n.color),
            (EffectKind::Noise(n), true) => Some(&mut n.secondary),
            _ => None,
        }
    }
    /// How far around the node a backdrop effect (background blur, glass) reads what is behind
    /// it, as a blur radius (document units); 0 for other effects.
    pub fn backdrop_radius(&self) -> f64 {
        if !self.visible {
            return 0.0;
        }
        let r = match &self.kind {
            EffectKind::BackgroundBlur(b) => b.max_radius(),
            // The frost blur plus the furthest bend, in blur radii (a blur reads 1.5 radii).
            EffectKind::Glass(g) => g.sanitized().frost + g.max_bend() / 1.5,
            _ => 0.0,
        };
        if r.is_finite() { r } else { 0.0 }
    }
}

/// Corner radii `[tl, tr, br, bl]` and smoothing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Corners {
    pub radii: [f64; 4],
    pub smoothing: f64,
}

impl Corners {
    pub fn uniform(&self) -> Option<f64> {
        let [a, b, c, d] = self.radii;
        if a == b && b == c && c == d { Some(a) } else { None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum HConstraint {
    #[default]
    Left,
    Right,
    LeftRight,
    Center,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum VConstraint {
    #[default]
    Top,
    Bottom,
    TopBottom,
    Center,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Constraints {
    pub horizontal: HConstraint,
    pub vertical: VConstraint,
}

/// Auto layout direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum LayoutMode {
    #[default]
    Horizontal,
    Vertical,
    Grid,
}

/// Alignment along the main axis. The `Space*` modes are the auto gaps: the free space goes
/// between the children only (between), equally between them and at the edges (evenly), or
/// between them with half as much at each edge (around).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MainAlign {
    #[default]
    Min,
    Center,
    Max,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

impl MainAlign {
    /// An auto gap (the gap comes from the free space, not the gap value).
    pub fn is_auto_gap(self) -> bool {
        matches!(self, MainAlign::SpaceBetween | MainAlign::SpaceAround | MainAlign::SpaceEvenly)
    }
}

/// A grid auto layout child's alignment in its cell on one axis (`None` where it is used: the
/// frame's alignment).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CellAlign {
    #[default]
    Min,
    Center,
    Max,
}

/// Alignment across the main axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CrossAlign {
    #[default]
    Min,
    Center,
    Max,
    Baseline,
}

/// How the lines of a wrapping layout (rows of a horizontal one, columns of a vertical one) share
/// the free space across the flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ContentAlign {
    /// Packed, `cross_gap` apart, at the start, centre or end the cross alignment names (lines of
    /// nothing but fill children stretch instead).
    #[default]
    Auto,
    /// The free space goes between the lines (one line, or no free space: like `Auto`).
    SpaceBetween,
}

/// Auto layout container properties (a frame with auto layout).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoLayout {
    pub mode: LayoutMode,
    pub wrap: bool,
    /// Gap between items along the main axis.
    pub gap: f64,
    /// Gap between wrapped rows/columns.
    pub cross_gap: f64,
    /// Padding `[top, right, bottom, left]`.
    pub padding: [f64; 4],
    pub main_align: MainAlign,
    pub cross_align: CrossAlign,
    /// Wrapping layouts: how the lines are spread across the flow.
    pub content_align: ContentAlign,
    /// Grid mode: column and row counts (rows grow to fit the children).
    pub grid_columns: u32,
    pub grid_rows: u32,
    /// Grid mode: column track sizes (missing tracks are `1fr`).
    pub column_tracks: Vec<GridTrack>,
    /// Grid mode: row track sizes (missing tracks are `1fr`).
    pub row_tracks: Vec<GridTrack>,
    /// The frame's own inside strokes take up space in the layout (like padding).
    pub strokes_included: bool,
    /// Canvas stacking: the first child paints on top (default: the last one does).
    pub reverse_z: bool,
}

impl Default for AutoLayout {
    fn default() -> Self {
        Self {
            mode: LayoutMode::Horizontal,
            wrap: false,
            gap: 10.0,
            cross_gap: 10.0,
            padding: [10.0; 4],
            main_align: MainAlign::Min,
            cross_align: CrossAlign::Min,
            content_align: ContentAlign::Auto,
            grid_columns: 2,
            grid_rows: 2,
            column_tracks: Vec::new(),
            row_tracks: Vec::new(),
            // New auto layout includes inside strokes.
            strokes_included: true,
            reverse_z: false,
        }
    }
}

/// How a node sizes on one axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Sizing {
    #[default]
    Fixed,
    /// Hug contents (auto layout frames, text).
    Hug,
    /// Fill the auto layout parent.
    Fill,
}

impl Sizing {
    pub fn label(self) -> &'static str {
        match self {
            Sizing::Fixed => "Fixed",
            Sizing::Hug => "Hug",
            Sizing::Fill => "Fill",
        }
    }
}

/// One grid auto layout track (a column or a row): a fixed size in px, hugging its contents, or
/// filling a share of the free space (`value` fractional units, like CSS `fr`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GridTrack {
    pub sizing: Sizing,
    /// Fixed: the size in px. Fill: the fractional units. Hug: unused.
    pub value: f64,
}

impl Default for GridTrack {
    fn default() -> Self {
        Self { sizing: Sizing::Fill, value: 1.0 }
    }
}

impl GridTrack {
    pub fn fixed(px: f64) -> Self {
        Self { sizing: Sizing::Fixed, value: px }
    }
    pub fn hug() -> Self {
        Self { sizing: Sizing::Hug, value: 0.0 }
    }
    pub fn fill(fr: f64) -> Self {
        Self { sizing: Sizing::Fill, value: fr }
    }
}

/// Layout properties a node has as a child (of an auto layout frame) or as a resizable box.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ChildLayout {
    pub sizing_h: Sizing,
    pub sizing_v: Sizing,
    /// Ignore auto layout (absolute position inside an auto layout frame).
    pub absolute: bool,
    pub min_w: Option<f64>,
    pub max_w: Option<f64>,
    pub min_h: Option<f64>,
    pub max_h: Option<f64>,
    /// Grid auto layout: anchor column / row (0-based); `None` = placed automatically.
    pub grid_column: Option<u32>,
    pub grid_row: Option<u32>,
    /// Grid auto layout: how many columns / rows the child spans (0 is read as 1).
    pub grid_column_span: u32,
    pub grid_row_span: u32,
    /// Grid auto layout: alignment in the cell, horizontally / vertically (`None`: the frame's).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grid_align_h: Option<CellAlign>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grid_align_v: Option<CellAlign>,
}

impl ChildLayout {
    /// Column span, at least 1.
    pub fn column_span(&self) -> u32 {
        self.grid_column_span.max(1)
    }
    /// Row span, at least 1.
    pub fn row_span(&self) -> u32 {
        self.grid_row_span.max(1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum GridKind {
    #[default]
    Grid,
    Columns,
    Rows,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum GridAlign {
    #[default]
    Stretch,
    Min,
    Center,
    Max,
}

/// A layout grid on a frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutGrid {
    pub kind: GridKind,
    /// Grid: cell size. Fixed (not stretched) columns/rows: their width/height.
    pub size: f64,
    /// Columns/rows: how many (0: Auto, as many fixed-size ones as fit; a stretched grid has 1).
    pub count: u32,
    pub gutter: f64,
    pub offset: f64,
    pub align: GridAlign,
    pub color: Color,
    pub visible: bool,
}

impl Default for LayoutGrid {
    fn default() -> Self {
        Self {
            kind: GridKind::Grid,
            size: 10.0,
            count: 5,
            gutter: 20.0,
            offset: 0.0,
            align: GridAlign::Stretch,
            color: Color::new(1.0, 0.0, 0.0, 0.1),
            visible: true,
        }
    }
}

impl LayoutGrid {
    pub fn columns() -> Self {
        Self { kind: GridKind::Columns, count: 5, gutter: 20.0, size: 0.0, ..Self::default() }
    }
    pub fn rows() -> Self {
        Self { kind: GridKind::Rows, count: 5, gutter: 20.0, size: 0.0, color: Color::new(0.0, 0.4, 1.0, 0.1), ..Self::default() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    #[default]
    Png,
    Jpg,
    Svg,
    Pdf,
    Webp,
    /// Android Vector Drawable (an `.xml` drawable resource).
    VectorDrawable,
}

impl ExportFormat {
    /// Every format, in the order the export menus list them.
    pub const ALL: [ExportFormat; 6] =
        [ExportFormat::Png, ExportFormat::Jpg, ExportFormat::Svg, ExportFormat::Pdf, ExportFormat::Webp, ExportFormat::VectorDrawable];

    /// Pixel formats (their size depends on the export scale).
    pub fn is_raster(self) -> bool {
        matches!(self, ExportFormat::Png | ExportFormat::Jpg | ExportFormat::Webp)
    }

    /// The name menus show ("PNG", "WebP").
    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Png => "PNG",
            ExportFormat::Jpg => "JPG",
            ExportFormat::Svg => "SVG",
            ExportFormat::Pdf => "PDF",
            ExportFormat::Webp => "WebP",
            ExportFormat::VectorDrawable => "XML",
        }
    }

    /// Formats whose output is vector (one file whatever the scale).
    pub fn is_vector(self) -> bool {
        matches!(self, ExportFormat::Svg | ExportFormat::Pdf | ExportFormat::VectorDrawable)
    }

    pub fn ext(self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::Jpg => "jpg",
            ExportFormat::Svg => "svg",
            ExportFormat::Pdf => "pdf",
            ExportFormat::Webp => "webp",
            ExportFormat::VectorDrawable => "xml",
        }
    }
    pub fn from_ext(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().trim_start_matches('.') {
            "png" => Some(ExportFormat::Png),
            "jpg" | "jpeg" => Some(ExportFormat::Jpg),
            "svg" => Some(ExportFormat::Svg),
            "pdf" => Some(ExportFormat::Pdf),
            "webp" => Some(ExportFormat::Webp),
            "xml" | "vd" | "vectordrawable" => Some(ExportFormat::VectorDrawable),
            _ => None,
        }
    }
}

/// Export size constraint.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "camelCase")]
pub enum ExportSize {
    Scale { value: f64 },
    Width { value: f64 },
    Height { value: f64 },
}

impl Default for ExportSize {
    fn default() -> Self {
        ExportSize::Scale { value: 1.0 }
    }
}

impl ExportSize {
    /// Scale presets offered by the export size menu.
    pub const SCALES: [f64; 7] = [0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0];
    /// Largest export scale.
    pub const MAX_SCALE: f64 = 64.0;
    /// Largest exported width or height.
    pub const MAX_PIXELS: f64 = 16384.0;

    /// Parse "2x", "0.5X", "@2x", "2" (scale), "512w" (width) or "512h" (height). Values must be
    /// positive and within the export limits.
    pub fn parse(s: &str) -> Option<ExportSize> {
        let s = s.trim().trim_start_matches('@').to_ascii_lowercase();
        let (num, unit) = match s.char_indices().last() {
            Some((i, c)) if c.is_ascii_alphabetic() => (s.get(..i)?.trim(), Some(c)),
            _ => (s.as_str(), None),
        };
        let value: f64 = num.parse().ok()?;
        match unit {
            None | Some('x') => ExportSize::Scale { value },
            Some('w') => ExportSize::Width { value },
            Some('h') => ExportSize::Height { value },
            _ => return None,
        }
        .checked()
    }

    /// The size when its value is positive and within the export limits.
    pub fn checked(self) -> Option<ExportSize> {
        let (value, max) = match self {
            ExportSize::Scale { value } => (value, Self::MAX_SCALE),
            ExportSize::Width { value } | ExportSize::Height { value } => (value, Self::MAX_PIXELS),
        };
        (value.is_finite() && value > 0.0 && value <= max).then_some(self)
    }

    /// "2x", "0.125x", "512w", "512h". Not rounded, so `parse` reads it back as the same size.
    pub fn label(self) -> String {
        // `{}` prints the shortest text that parses back to the same `f64`, and whole numbers
        // without a fraction ("2", not "2.0").
        match self {
            ExportSize::Scale { value } => format!("{value}x"),
            ExportSize::Width { value } => format!("{value}w"),
            ExportSize::Height { value } => format!("{value}h"),
        }
    }

    /// The scale that gives a `w` × `h` box this size (1 for an empty box).
    pub fn scale_for(self, w: f64, h: f64) -> f64 {
        let s = match self {
            ExportSize::Scale { value } => value,
            ExportSize::Width { value } if w > 0.0 => value / w,
            ExportSize::Height { value } if h > 0.0 => value / h,
            _ => 1.0,
        };
        if s.is_finite() && s > 0.0 { s.min(Self::MAX_SCALE) } else { 1.0 }
    }
}

/// Print settings of a PDF export: bleed, printer's marks and CMYK colour.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct PrintSettings {
    /// Bleed around each page, in points (one point per pixel): what lies past the page's edge
    /// is printed this far out, to be trimmed off.
    pub bleed: f64,
    /// Crop marks outside the bleed, at the trim's corners.
    pub crop_marks: bool,
    /// Bleed marks outside the bleed, at the bleed's corners.
    pub bleed_marks: bool,
    /// CMYK colour (converted from RGB) instead of RGB.
    pub cmyk: bool,
}

impl PrintSettings {
    /// Largest bleed, in points (two inches).
    pub const MAX_BLEED: f64 = 144.0;

    pub fn is_default(&self) -> bool {
        *self == PrintSettings::default()
    }

    /// The bleed made safe: finite, within `0..=MAX_BLEED`.
    pub fn bleed(&self) -> f64 {
        if self.bleed.is_finite() { self.bleed.clamp(0.0, Self::MAX_BLEED) } else { 0.0 }
    }

    pub fn marks(&self) -> bool {
        self.crop_marks || (self.bleed_marks && self.bleed() > 0.0)
    }

    /// Whether the page gets a bleed or marks around its trim.
    pub fn extends_page(&self) -> bool {
        self.bleed() > 0.0 || self.marks()
    }

    /// A length in points: a number (points), or with a unit: `pt`, `px` (a point), `mm`, `cm`,
    /// `in` or `"` ("3mm", "0.125 in", "9pt"). `None` unless finite and not negative.
    pub fn parse_length(s: &str) -> Option<f64> {
        let s = s.trim().to_ascii_lowercase();
        let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e')).unwrap_or(s.len());
        let (num, unit) = (s.get(..split)?.trim(), s.get(split..)?.trim());
        let v: f64 = num.parse().ok()?;
        let k = match unit {
            "" | "pt" | "px" => 1.0,
            "mm" => 72.0 / 25.4,
            "cm" => 72.0 / 2.54,
            "in" | "\"" => 72.0,
            _ => return None,
        };
        let pt = v * k;
        (pt.is_finite() && pt >= 0.0).then_some(pt)
    }

    /// A length for people: in millimetres when it's a whole tenth of one, else in points.
    pub fn length_label(pt: f64) -> String {
        let mm = pt * 25.4 / 72.0;
        if ((mm * 10.0).round() - mm * 10.0).abs() < 1e-6 {
            format!("{} mm", crate::text::trim_num((mm * 10.0).round() / 10.0))
        } else {
            format!("{} pt", crate::text::trim_num(pt))
        }
    }
}

/// One row of a node's Export section.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExportSetting {
    pub size: ExportSize,
    pub suffix: String,
    pub format: ExportFormat,
    /// PDF only: bleed, marks and colour.
    #[serde(skip_serializing_if = "PrintSettings::is_default")]
    pub print: PrintSettings,
    /// Only the layer itself (on), or every layer that shows in its area (off).
    #[serde(skip_serializing_if = "is_true")]
    pub contents_only: bool,
    /// The layer's own box, cutting off what it paints outside it (shadows, outside strokes).
    #[serde(skip_serializing_if = "is_false")]
    pub include_bounding_box: bool,
    /// SVG: text as outlines (on) or as `<text>` elements (off).
    #[serde(skip_serializing_if = "is_true")]
    pub svg_outline_text: bool,
    /// SVG: layer names as `id` attributes.
    #[serde(skip_serializing_if = "is_false")]
    pub svg_id_attribute: bool,
    /// SVG: strokes as `stroke` attributes where that's exact (on), else as outlines.
    #[serde(skip_serializing_if = "is_true")]
    pub svg_simplify_stroke: bool,
    /// Android Vector Drawable: what to do with what a drawable can't hold exactly.
    #[serde(skip_serializing_if = "VdCompat::is_default")]
    pub vd_compat: VdCompat,
}

/// How an Android Vector Drawable export handles what the format can't draw exactly (layer
/// opacity over overlapping shapes, even-odd fills, diamond gradients, alpha masks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum VdCompat {
    /// The closest drawing, with a warning for each approximation.
    #[default]
    BestEffort,
    /// Never approximate: what can't be drawn exactly is left out, with a warning.
    Exact,
    /// Write what maps, leave out the rest, no warnings.
    Ignore,
}

impl VdCompat {
    pub const ALL: [VdCompat; 3] = [VdCompat::BestEffort, VdCompat::Exact, VdCompat::Ignore];

    fn is_default(&self) -> bool {
        *self == VdCompat::BestEffort
    }

    /// The name commands, files and the CLI use ("bestEffort").
    pub fn key(self) -> &'static str {
        match self {
            VdCompat::BestEffort => "bestEffort",
            VdCompat::Exact => "exact",
            VdCompat::Ignore => "ignore",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.key() == s)
    }
}

impl Default for ExportSetting {
    fn default() -> Self {
        Self {
            size: ExportSize::default(),
            suffix: String::new(),
            format: ExportFormat::default(),
            print: PrintSettings::default(),
            contents_only: true,
            include_bounding_box: false,
            svg_outline_text: true,
            svg_id_attribute: false,
            svg_simplify_stroke: true,
            vd_compat: VdCompat::BestEffort,
        }
    }
}

fn is_true(b: &bool) -> bool {
    *b
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl ExportSetting {
    /// The row adding a setting after `last` creates: 1x first, then one scale step up with a
    /// matching "@2x"-style suffix.
    pub fn next_after(last: Option<&ExportSetting>) -> ExportSetting {
        match last.map(|e| e.size) {
            Some(ExportSize::Scale { value }) => {
                let size = ExportSize::Scale { value: (value.floor() + 1.0).min(ExportSize::MAX_SCALE) };
                ExportSetting { suffix: format!("@{}", size.label()), size, format: ExportFormat::Png, ..Default::default() }
            }
            _ => ExportSetting::default(),
        }
    }

    /// What the exported file name gets after the layer name: the suffix, else "@2x"-style for
    /// raster exports at another scale.
    pub fn file_suffix(&self) -> String {
        match self.size {
            _ if !self.suffix.is_empty() => self.suffix.clone(),
            ExportSize::Scale { value } if (value - 1.0).abs() > 1e-9 && self.format.is_raster() => format!("@{}", self.size.label()),
            _ => String::new(),
        }
    }
}
