//! FigCraft colour and paint.
//!
//! [`Color`] is non-premultiplied sRGB in `0..=1` floats (what a design panel shows as hex + opacity).
//! [`Paint`] is one entry of a node's fill or stroke list: solid, the four gradient kinds, an
//! image or a pattern (a layer tiled), each with its own opacity, visibility and blend mode. [`ColorProfile`] says which RGB
//! space (sRGB or Display P3) a document's values are in and converts between them.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use serde::{Deserialize, Serialize};

mod blend;
mod hsx;
mod icc;
mod lch;
mod named;
mod profile;
pub use blend::BlendMode;
pub use hsx::*;
pub use lch::{Lch, Oklch};
pub use profile::{ColorProfile, ProfileConversion};

/// A non-premultiplied sRGB colour with alpha, components in `0..=1`. Deserializes from
/// `{r, g, b, a}`, `[r, g, b(, a)]`, or a CSS/hex string (`"#0D99FF"`, `"rgba(…)"`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    #[serde(default = "one")]
    pub a: f32,
}

fn one() -> f32 {
    1.0
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ColorRepr {
    Obj {
        r: f32,
        g: f32,
        b: f32,
        #[serde(default = "one")]
        a: f32,
    },
    Arr(Vec<f32>),
    Text(String),
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        match ColorRepr::deserialize(d)? {
            ColorRepr::Obj { r, g, b, a } => Ok(Color::new(r, g, b, a)),
            ColorRepr::Arr(v) => match v.as_slice() {
                [r, g, b] => Ok(Color::new(*r, *g, *b, 1.0)),
                [r, g, b, a] => Ok(Color::new(*r, *g, *b, *a)),
                _ => Err(serde::de::Error::custom("colour arrays have 3 or 4 components")),
            },
            ColorRepr::Text(t) => Color::parse_css(&t).ok_or_else(|| serde::de::Error::custom(format!("not a colour: {t}"))),
        }
    }
}

fn clamp01(v: f32) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
}

impl Default for Color {
    fn default() -> Self {
        Color::BLACK
    }
}

impl Color {
    pub const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const TRANSPARENT: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    /// Figma's default shape fill (#D9D9D9).
    pub const DEFAULT_SHAPE: Color = Color::from_rgb8_const(0xD9, 0xD9, 0xD9);
    /// Default page background (#F5F5F5).
    pub const DEFAULT_PAGE: Color = Color::from_rgb8_const(0xF5, 0xF5, 0xF5);
    /// The background of a page made while the interface shows its dark theme (#1E1E1E).
    pub const DARK_PAGE: Color = Color::from_rgb8_const(0x1E, 0x1E, 0x1E);

    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r: clamp01(r), g: clamp01(g), b: clamp01(b), a: clamp01(a) }
    }

    pub const fn from_rgb8_const(r: u8, g: u8, b: u8) -> Self {
        Self { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
    }

    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r: f32::from(r) / 255.0, g: f32::from(g) / 255.0, b: f32::from(b) / 255.0, a: f32::from(a) / 255.0 }
    }

    pub fn to_rgba8(self) -> [u8; 4] {
        let q = |v: f32| (clamp01(v) * 255.0).round() as u8;
        [q(self.r), q(self.g), q(self.b), q(self.a)]
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self { a: clamp01(a), ..self }
    }

    /// Clamp every component into range and replace NaN (untrusted input).
    pub fn sanitized(self) -> Self {
        Self::new(self.r, self.g, self.b, self.a)
    }

    /// `RRGGBB` (upper-case, no `#`), the way the design panel shows colours.
    pub fn to_hex(self) -> String {
        let [r, g, b, _] = self.to_rgba8();
        format!("{r:02X}{g:02X}{b:02X}")
    }

    /// `#RRGGBB` or `#RRGGBBAA` when not opaque.
    pub fn to_css_hex(self) -> String {
        let [r, g, b, a] = self.to_rgba8();
        if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{r:02X}{g:02X}{b:02X}{a:02X}") }
    }

    /// Parse hex the way the colour field does: `#` optional, 1, 2, 3, 4, 6 or 8 digits
    /// (`F` → `FFFFFF`, `AB` → `ABABAB`, `ABC` → `AABBCC`).
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_prefix('#').unwrap_or(s);
        if !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let digits: Vec<u8> = s.chars().filter_map(|c| c.to_digit(16)).map(|d| d as u8).collect();
        let byte = |hi: u8, lo: u8| hi * 16 + lo;
        let (r, g, b, a) = match digits.as_slice() {
            [d] => (byte(*d, *d), byte(*d, *d), byte(*d, *d), 255),
            [a, b] => (byte(*a, *b), byte(*a, *b), byte(*a, *b), 255),
            [r, g, b] => (byte(*r, *r), byte(*g, *g), byte(*b, *b), 255),
            [r, g, b, a] => (byte(*r, *r), byte(*g, *g), byte(*b, *b), byte(*a, *a)),
            [r1, r2, g1, g2, b1, b2] => (byte(*r1, *r2), byte(*g1, *g2), byte(*b1, *b2), 255),
            [r1, r2, g1, g2, b1, b2, a1, a2] => (byte(*r1, *r2), byte(*g1, *g2), byte(*b1, *b2), byte(*a1, *a2)),
            _ => return None,
        };
        Some(Self::from_rgba8(r, g, b, a))
    }

    /// What a colour field accepts: hex as [`Color::from_hex`] reads it, or a CSS colour name
    /// (`red` → `FF0000`, `Cornflower Blue` → `6495ED`; opaque).
    pub fn from_hex_or_name(s: &str) -> Option<Self> {
        Self::from_hex(s).or_else(|| named::lookup(s).map(|[r, g, b]| Self::from_rgba8(r, g, b, 255)))
    }

    /// Parse a CSS colour: hex, `rgb()/rgba()`, `hsl()/hsla()`, `lch()`, `oklch()`, a CSS colour
    /// name or `transparent`. LCH and OKLCH outside sRGB come back as the nearest sRGB colour.
    pub fn parse_css(s: &str) -> Option<Self> {
        let t = s.trim().to_ascii_lowercase();
        if t.starts_with('#') {
            return Self::from_hex(&t);
        }
        if let Some(inner) = t.strip_prefix("rgba(").or_else(|| t.strip_prefix("rgb(")).and_then(|r| r.strip_suffix(')')) {
            let parts = css_args(inner)?;
            let num = |i: usize| -> Option<f32> {
                let p = parts.get(i)?;
                if let Some(pc) = p.strip_suffix('%') {
                    pc.trim().parse::<f32>().ok().map(|v| v / 100.0)
                } else {
                    p.parse::<f32>().ok().map(|v| v / 255.0)
                }
            };
            let alpha = match parts.get(3) {
                Some(a) => parse_alpha(a)?,
                None => 1.0,
            };
            if parts.len() > 4 {
                return None;
            }
            return Some(Self::new(num(0)?, num(1)?, num(2)?, alpha));
        }
        if let Some(inner) = t.strip_prefix("hsla(").or_else(|| t.strip_prefix("hsl(")).and_then(|r| r.strip_suffix(')')) {
            let parts = css_args(inner)?;
            let h = parts.first()?.trim_end_matches("deg").parse::<f32>().ok()?;
            let s = parts.get(1)?.trim_end_matches('%').parse::<f32>().ok()? / 100.0;
            let l = parts.get(2)?.trim_end_matches('%').parse::<f32>().ok()? / 100.0;
            if !(h.is_finite() && s.is_finite() && l.is_finite()) {
                return None;
            }
            let alpha = match parts.get(3) {
                Some(a) => parse_alpha(a)?,
                None => 1.0,
            };
            if parts.len() > 4 {
                return None;
            }
            return Some(Hsl { h, s, l }.to_color().with_alpha(alpha));
        }
        // Percentages per CSS Color 4: OKLCH 100% is lightness 1, chroma 0.4; LCH 100% is 100 and 150.
        if let Some(inner) = t.strip_prefix("oklch(").and_then(|r| r.strip_suffix(')')) {
            let ([l, c, h], alpha) = css_polar(inner, 1.0, 0.4)?;
            return Some(Oklch { l, c, h }.to_color().with_alpha(alpha));
        }
        if let Some(inner) = t.strip_prefix("lch(").and_then(|r| r.strip_suffix(')')) {
            let ([l, c, h], alpha) = css_polar(inner, 100.0, 150.0)?;
            return Some(Lch { l, c, h }.to_color().with_alpha(alpha));
        }
        if t == "transparent" {
            return Some(Color::TRANSPARENT);
        }
        Self::from_hex_or_name(&t)
    }

    /// CSS `rgba(...)` / hex text for copy-as-CSS.
    pub fn to_css(self) -> String {
        let [r, g, b, _] = self.to_rgba8();
        if self.a >= 0.999 {
            return self.to_css_hex();
        }
        // Two decimals when they keep the 8-bit alpha, three otherwise (near-transparent colours).
        let a8 = |a: f32| (a.clamp(0.0, 1.0) * 255.0).round() as u8;
        let short = format!("{:.2}", self.a);
        let alpha = if short.parse::<f32>().is_ok_and(|v| a8(v) == a8(self.a)) { short } else { format!("{:.3}", self.a) };
        format!("rgba({r}, {g}, {b}, {alpha})")
    }

    /// Linear interpolation in sRGB (as gradients render).
    pub fn lerp(self, other: Color, t: f32) -> Color {
        let t = clamp01(t);
        Color::new(self.r + (other.r - self.r) * t, self.g + (other.g - self.g) * t, self.b + (other.b - self.b) * t, self.a + (other.a - self.a) * t)
    }

    /// Relative luminance (WCAG).
    pub fn luminance(self) -> f32 {
        let lin = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    /// Whether black on this colour reads worse than white (a dark page: new text is white).
    pub fn is_dark(self) -> bool {
        self.contrast(Color::WHITE) > self.contrast(Color::BLACK)
    }

    /// WCAG contrast ratio between two opaque colours.
    pub fn contrast(self, other: Color) -> f32 {
        let (a, b) = (self.luminance() + 0.05, other.luminance() + 0.05);
        if a > b { a / b } else { b / a }
    }
}

/// Split CSS function arguments on `,` and `/` (each may be surrounded by spaces) and on runs of
/// whitespace. An empty argument between two separators makes the whole list invalid.
fn css_args(inner: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for seg in inner.replace('/', ",").split(',') {
        let before = out.len();
        out.extend(seg.split_whitespace().map(str::to_string));
        if out.len() == before {
            return None;
        }
    }
    Some(out)
}

/// The arguments of `lch()` / `oklch()`: lightness, chroma, hue and alpha. Percentages are of
/// `l_full` / `c_full`; `none` is 0.
fn css_polar(inner: &str, l_full: f32, c_full: f32) -> Option<([f32; 3], f32)> {
    let parts = css_args(inner)?;
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let l = css_number(parts.first()?, l_full)?;
    let c = css_number(parts.get(1)?, c_full)?;
    let h = css_hue(parts.get(2)?)?;
    let alpha = match parts.get(3).map(String::as_str) {
        None | Some("none") => 1.0,
        Some(a) => parse_alpha(a)?,
    };
    Some(([l, c, h], alpha))
}

/// A CSS number, a percentage of `full` or `none` (0); finite only.
fn css_number(p: &str, full: f32) -> Option<f32> {
    let v = match p.strip_suffix('%') {
        _ if p == "none" => 0.0,
        Some(pc) => pc.trim().parse::<f32>().ok()? / 100.0 * full,
        None => p.parse::<f32>().ok()?,
    };
    v.is_finite().then_some(v)
}

/// A CSS hue in degrees: a number, `deg`, `rad`, `grad`, `turn` or `none` (0); finite only.
fn css_hue(p: &str) -> Option<f32> {
    if p == "none" {
        return Some(0.0);
    }
    let (num, per_degree) = [("grad", 400.0 / 360.0), ("turn", 1.0 / 360.0), ("rad", std::f32::consts::PI / 180.0), ("deg", 1.0)]
        .into_iter()
        .find_map(|(unit, k)| p.strip_suffix(unit).map(|n| (n, k)))
        .unwrap_or((p, 1.0));
    let v = num.parse::<f32>().ok()? / per_degree;
    v.is_finite().then_some(v)
}

fn parse_alpha(a: &str) -> Option<f32> {
    let v = if let Some(pc) = a.strip_suffix('%') { pc.parse::<f32>().ok().map(|v| v / 100.0) } else { a.parse::<f32>().ok() }?;
    v.is_finite().then_some(v)
}

/// One colour stop of a gradient.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorStop {
    /// Position along the gradient, 0..=1.
    pub position: f32,
    pub color: Color,
}

/// The gradient kinds Figma offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum GradientKind {
    #[default]
    Linear,
    Radial,
    Angular,
    Diamond,
}

/// A gradient paint. The three handles are in the node's unit box (0..1 on both axes): `from`
/// is the start (centre for radial/angular/diamond), `to` the end of the main axis, `width` the end
/// of the secondary axis (ellipse width for radial).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub kind: GradientKind,
    pub stops: Vec<ColorStop>,
    pub from: (f64, f64),
    pub to: (f64, f64),
    pub width: (f64, f64),
}

impl Gradient {
    /// Figma's default gradient for `kind`: black → transparent black? No: the default stop pair is
    /// the current colour at 100% → the same at 0% opacity. We use black→white for a visible default.
    pub fn default_for(kind: GradientKind, base: Color) -> Self {
        let (from, to, width) = match kind {
            // The width handle square to the axis from the start (colour bands across the axis).
            GradientKind::Linear => ((0.5, 0.0), (0.5, 1.0), (-0.5, 0.0)),
            _ => ((0.5, 0.5), (0.5, 1.0), (1.0, 0.5)),
        };
        Self {
            kind,
            stops: vec![ColorStop { position: 0.0, color: base.with_alpha(1.0) }, ColorStop { position: 1.0, color: base.with_alpha(0.0) }],
            from,
            to,
            width,
        }
    }

    /// Put a linear gradient's width handle square to its axis in a `w`×`h` box, so its colour
    /// bands cross the axis at right angles on screen (the canvas has no handle for it; imported
    /// files can still skew it). Moving the start or end handle calls this.
    pub fn square_linear_width(&mut self, w: f64, h: f64) {
        let (w, h) = if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 { (w, h) } else { (1.0, 1.0) };
        let (dx, dy) = ((self.to.0 - self.from.0) * w, (self.to.1 - self.from.1) * h);
        self.width = (self.from.0 - dy / w, self.from.1 + dx / h);
    }

    /// Stops sorted by position with NaNs removed; at least one stop.
    pub fn sorted_stops(&self) -> Vec<ColorStop> {
        let mut s: Vec<ColorStop> = self
            .stops
            .iter()
            .filter(|s| s.position.is_finite())
            .map(|s| ColorStop { position: clamp01(s.position), color: s.color.sanitized() })
            .collect();
        s.sort_by(|a, b| a.position.total_cmp(&b.position));
        if s.is_empty() {
            s.push(ColorStop { position: 0.0, color: Color::BLACK });
        }
        s
    }

    /// The colour at `t` (0..1) along the gradient.
    pub fn sample(&self, t: f32) -> Color {
        let stops = self.sorted_stops();
        let t = clamp01(t);
        let mut prev = match stops.first() {
            Some(s) => *s,
            None => return Color::BLACK,
        };
        if t <= prev.position {
            return prev.color;
        }
        for s in stops.iter().skip(1) {
            if t <= s.position {
                let span = s.position - prev.position;
                let f = if span > 1e-6 { (t - prev.position) / span } else { 1.0 };
                return prev.color.lerp(s.color, f);
            }
            prev = *s;
        }
        prev.color
    }
}

/// How an image fills its node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ImageScaleMode {
    #[default]
    Fill,
    Fit,
    Crop,
    Tile,
}

/// Image adjustments (all `-1..=1`, 0 = unchanged).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ImageFilters {
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub temperature: f32,
    pub tint: f32,
    pub highlights: f32,
    pub shadows: f32,
}

impl ImageFilters {
    pub fn is_identity(&self) -> bool {
        *self == ImageFilters::default()
    }

    /// The built-in adjustment presets: `(id, label, adjustments)`. "Reset" is the identity.
    pub const PRESETS: [(&str, &str, ImageFilters); 7] = [
        ("vivid", "Vivid", ImageFilters { contrast: 0.2, saturation: 0.45, highlights: -0.1, shadows: 0.1, ..ImageFilters::NONE }),
        ("muted", "Muted", ImageFilters { contrast: -0.15, saturation: -0.45, ..ImageFilters::NONE }),
        ("warm", "Warm", ImageFilters { temperature: 0.5, tint: 0.1, saturation: 0.1, ..ImageFilters::NONE }),
        ("cool", "Cool", ImageFilters { temperature: -0.5, tint: -0.05, ..ImageFilters::NONE }),
        ("blackAndWhite", "Black & white", ImageFilters { saturation: -1.0, contrast: 0.15, ..ImageFilters::NONE }),
        ("fade", "Fade", ImageFilters { contrast: -0.35, saturation: -0.2, shadows: 0.35, highlights: -0.1, ..ImageFilters::NONE }),
        ("reset", "Reset", ImageFilters::NONE),
    ];

    /// No adjustment (a `const` [`Default`]).
    pub const NONE: ImageFilters =
        ImageFilters { exposure: 0.0, contrast: 0.0, saturation: 0.0, temperature: 0.0, tint: 0.0, highlights: 0.0, shadows: 0.0 };

    /// The built-in preset `name` (its id or label, ignoring case, spaces, `-`, `_` and `&`).
    pub fn preset(name: &str) -> Option<ImageFilters> {
        let key = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
        let k = key(name);
        Self::PRESETS.iter().find(|(id, label, _)| key(id) == k || key(label) == k).map(|(_, _, f)| *f)
    }

    /// Every value clamped into `-1..=1` (NaN → 0).
    pub fn sanitized(self) -> Self {
        let c = |v: f32| if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 };
        Self {
            exposure: c(self.exposure),
            contrast: c(self.contrast),
            saturation: c(self.saturation),
            temperature: c(self.temperature),
            tint: c(self.tint),
            highlights: c(self.highlights),
            shadows: c(self.shadows),
        }
    }
}

/// An image paint: the image is referenced by its content hash in the document's image store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImagePaint {
    pub image: String,
    #[serde(default)]
    pub scale_mode: ImageScaleMode,
    /// Tile scale (Tile mode), 1 = the image's own size.
    #[serde(default = "one64")]
    pub scale: f64,
    /// Rotation in multiples of 90°.
    #[serde(default)]
    pub rotation: i32,
    /// Crop transform (Crop mode): the unit-box → image-uv mapping as a 2×3 matrix.
    #[serde(default)]
    pub crop: Option<[f64; 6]>,
    #[serde(default)]
    pub filters: ImageFilters,
}

impl ImagePaint {
    /// Size of the image after its 90° rotation.
    pub fn rotated_size(&self, iw: f64, ih: f64) -> (f64, f64) {
        if self.rotation.rem_euclid(2) == 1 { (ih, iw) } else { (iw, ih) }
    }

    /// The crop transform in effect: the stored one, or (when unset) the Fill placement so that
    /// switching to Crop keeps the picture where it was.
    pub fn effective_crop(&self, iw: f64, ih: f64, w: f64, h: f64) -> [f64; 6] {
        if let Some(c) = self.crop
            && c.iter().all(|v| v.is_finite())
            && c[0].abs() > 1e-9
            && c[3].abs() > 1e-9
        {
            return c;
        }
        self.fitted_crop(iw, ih, w, h, false)
    }

    /// The crop transform that keeps the picture where its scale mode draws it: the crop in effect
    /// in Crop mode, the Fit placement in Fit mode, else (Fill, Tile) the Fill placement.
    pub fn placement_crop(&self, iw: f64, ih: f64, w: f64, h: f64) -> [f64; 6] {
        match self.scale_mode {
            ImageScaleMode::Crop => self.effective_crop(iw, ih, w, h),
            mode => self.fitted_crop(iw, ih, w, h, mode == ImageScaleMode::Fit),
        }
    }

    /// The Fill placement (`fit`: the Fit placement) as a crop transform.
    fn fitted_crop(&self, iw: f64, ih: f64, w: f64, h: f64, fit: bool) -> [f64; 6] {
        let (rw, rh) = self.rotated_size(iw.max(1.0), ih.max(1.0));
        let (w, h) = (w.max(1e-6), h.max(1e-6));
        let s = if fit { (w / rw).min(h / rh) } else { (w / rw).max(h / rh) };
        let (x0, y0) = ((w - rw * s) / 2.0, (h - rh * s) / 2.0);
        [w / (rw * s), 0.0, 0.0, h / (rh * s), -x0 / (rw * s), -y0 / (rh * s)]
    }

    /// The image's rectangle in the node's unit box `(x0, y0, x1, y1)` for an axis-aligned crop.
    pub fn crop_rect(crop: [f64; 6]) -> (f64, f64, f64, f64) {
        let (a, d, e, f) = (crop[0], crop[3], crop[4], crop[5]);
        (-e / a, -f / d, (1.0 - e) / a, (1.0 - f) / d)
    }

    /// The crop transform that places the image at `(x0, y0, x1, y1)` in the unit box.
    pub fn crop_from_rect(x0: f64, y0: f64, x1: f64, y1: f64) -> [f64; 6] {
        let a = 1.0 / (x1 - x0).abs().max(1e-6);
        let d = 1.0 / (y1 - y0).abs().max(1e-6);
        let (x0, y0) = (x0.min(x1), y0.min(y1));
        [a, 0.0, 0.0, d, -x0 * a, -y0 * d]
    }
}

fn one64() -> f64 {
    1.0
}

/// How pattern tiles are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PatternTile {
    /// Rows and columns.
    #[default]
    Grid,
    /// Every other row shifted by half a tile.
    HorizontalHex,
    /// Every other column shifted by half a tile.
    VerticalHex,
}

/// Where the tile grid starts in the filled box: a tile in its top-left corner, centre or
/// bottom-right corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PatternAlign {
    #[default]
    Start,
    Center,
    End,
}

/// A pattern paint: a layer of the document (`source`, a node id) drawn as a tile, repeated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PatternPaint {
    pub source: u64,
    #[serde(default)]
    pub tile: PatternTile,
    /// Tile scale, 1 = the source's own size.
    #[serde(default = "one64")]
    pub scale: f64,
    /// Gap between tiles (horizontal, vertical), in document units.
    #[serde(default)]
    pub spacing: (f64, f64),
    #[serde(default)]
    pub align: PatternAlign,
}

/// One repeating cell of a pattern in the filled box's space: `size` repeats from `origin`, and
/// the source is drawn at each of `tiles` (its top-left), scaled by `scale`.
#[derive(Clone, Debug, PartialEq)]
pub struct PatternCell {
    pub size: (f64, f64),
    pub origin: (f64, f64),
    pub tiles: Vec<(f64, f64)>,
    pub scale: f64,
}

impl PatternPaint {
    /// The smallest and largest tile scale.
    pub const SCALE_RANGE: (f64, f64) = (0.01, 100.0);
    /// The largest gap between tiles.
    pub const MAX_SPACING: f64 = 10_000.0;

    pub fn new(source: u64) -> Self {
        Self { source, tile: PatternTile::Grid, scale: 1.0, spacing: (0.0, 0.0), align: PatternAlign::Start }
    }

    /// The cell for a `tw`×`th` source filling a `w`×`h` box; `None` when the source has no size.
    pub fn cell(&self, tw: f64, th: f64, w: f64, h: f64) -> Option<PatternCell> {
        let (lo, hi) = Self::SCALE_RANGE;
        let scale = if self.scale.is_finite() { self.scale.clamp(lo, hi) } else { 1.0 };
        let gap = |v: f64| if v.is_finite() { v.clamp(0.0, Self::MAX_SPACING) } else { 0.0 };
        let (tw, th) = (tw * scale, th * scale);
        if !(tw.is_finite() && th.is_finite() && tw > 1e-6 && th > 1e-6) {
            return None;
        }
        let (px, py) = (tw + gap(self.spacing.0), th + gap(self.spacing.1));
        let (size, tiles) = match self.tile {
            PatternTile::Grid => ((px, py), vec![(0.0, 0.0)]),
            // The shifted row (column) is drawn on both sides so it wraps across the cell's edge.
            PatternTile::HorizontalHex => ((px, py * 2.0), vec![(0.0, 0.0), (px / 2.0, py), (-px / 2.0, py)]),
            PatternTile::VerticalHex => ((px * 2.0, py), vec![(0.0, 0.0), (px, py / 2.0), (px, -py / 2.0)]),
        };
        let k = match self.align {
            PatternAlign::Start => 0.0,
            PatternAlign::Center => 0.5,
            PatternAlign::End => 1.0,
        };
        let fin = |v: f64| if v.is_finite() { v } else { 0.0 };
        Some(PatternCell { size, origin: (fin((w - tw) * k), fin((h - th) * k)), tiles, scale })
    }
}

/// What a paint paints with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PaintKind {
    Solid { color: Color },
    Gradient(Gradient),
    Image(ImagePaint),
    Pattern(PatternPaint),
}

/// One fill or stroke paint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Paint {
    #[serde(flatten)]
    pub kind: PaintKind,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub blend: BlendMode,
    /// The part of the opacity that is the alpha of a typed solid colour (see [`Paint::solid`]),
    /// the rest being an opacity set on its own: a colour variable bound to the paint replaces
    /// this part only. 1: all of it is the paint's own.
    #[serde(default = "one", rename = "colorAlpha", skip_serializing_if = "is_one")]
    pub color_alpha: f32,
}

/// Paints compare by what they paint; where the opacity came from doesn't matter.
impl PartialEq for Paint {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.opacity == other.opacity && self.visible == other.visible && self.blend == other.blend
    }
}

fn yes() -> bool {
    true
}

fn is_one(x: &f32) -> bool {
    *x == 1.0
}

impl Paint {
    pub fn solid(color: Color) -> Self {
        Self {
            kind: PaintKind::Solid { color: color.with_alpha(1.0) },
            opacity: color.a,
            visible: true,
            blend: BlendMode::Normal,
            color_alpha: color.a,
        }
    }

    pub fn gradient(g: Gradient) -> Self {
        Self { kind: PaintKind::Gradient(g), opacity: 1.0, visible: true, blend: BlendMode::Normal, color_alpha: 1.0 }
    }

    pub fn image(hash: impl Into<String>, mode: ImageScaleMode) -> Self {
        Self {
            kind: PaintKind::Image(ImagePaint {
                image: hash.into(),
                scale_mode: mode,
                scale: 1.0,
                rotation: 0,
                crop: None,
                filters: ImageFilters::default(),
            }),
            opacity: 1.0,
            visible: true,
            blend: BlendMode::Normal,
            color_alpha: 1.0,
        }
    }

    /// Layer `source` (a node id) tiled at its own size.
    pub fn pattern(source: u64) -> Self {
        Self { kind: PaintKind::Pattern(PatternPaint::new(source)), opacity: 1.0, visible: true, blend: BlendMode::Normal, color_alpha: 1.0 }
    }

    /// The solid colour (with the paint opacity folded into alpha), if solid.
    pub fn solid_color(&self) -> Option<Color> {
        match &self.kind {
            PaintKind::Solid { color } => Some(color.with_alpha(color.a * self.opacity)),
            _ => None,
        }
    }

    /// A short label for the panel row ("D9D9D9", "Linear", "Image").
    pub fn label(&self) -> String {
        match &self.kind {
            PaintKind::Solid { color } => color.to_hex(),
            PaintKind::Gradient(g) => match g.kind {
                GradientKind::Linear => "Linear".into(),
                GradientKind::Radial => "Radial".into(),
                GradientKind::Angular => "Angular".into(),
                GradientKind::Diamond => "Diamond".into(),
            },
            PaintKind::Image(_) => "Image".into(),
            PaintKind::Pattern(_) => "Pattern".into(),
        }
    }

    /// Visit every colour of this paint (the solid colour or the gradient stops).
    pub fn for_each_color_mut(&mut self, f: &mut dyn FnMut(&mut Color)) {
        match &mut self.kind {
            PaintKind::Solid { color } => f(color),
            PaintKind::Gradient(g) => g.stops.iter_mut().for_each(|s| f(&mut s.color)),
            PaintKind::Image(_) | PaintKind::Pattern(_) => {}
        }
    }

    /// A representative colour (for swatches and outlines).
    pub fn preview_color(&self) -> Color {
        match &self.kind {
            PaintKind::Solid { color } => color.with_alpha(color.a * self.opacity),
            PaintKind::Gradient(g) => g.sample(0.0),
            PaintKind::Image(_) | PaintKind::Pattern(_) => Color::from_rgb8_const(0xB3, 0xB3, 0xB3),
        }
    }
}

#[cfg(test)]
mod gradient_tests;
#[cfg(test)]
mod lch_tests;
#[cfg(test)]
mod pattern_tests;
#[cfg(test)]
mod profile_tests;
#[cfg(test)]
mod tests;
