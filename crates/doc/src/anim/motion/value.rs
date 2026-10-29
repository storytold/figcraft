//! Animatable properties and their typed values.

use figcraft_color::Color;
use figcraft_geom::Vec2;
use serde::{Deserialize, Serialize};

/// A node property that keyframes can animate.
///
/// Units are model units, not display units: [`AnimProperty::display_factor`] converts opacity
/// and scale to the percentages the inspector shows. Rotation is in degrees, positive
/// counter-clockwise, the same number the Rotation field shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AnimProperty {
    /// Layout position (x, y) of the node's unrotated, unscaled box in its parent, in px.
    Position,
    /// Scale factor (x, y) relative to the node's static transform, 1.0 = 100%. Applied about the
    /// node's centre; negative values mirror.
    Scale,
    /// Rotation in degrees, counter-clockwise positive, about the node's centre.
    Rotation,
    Width,
    Height,
    /// 0.0 ..= 1.0.
    Opacity,
    /// Uniform corner radius in px; sets all four corners.
    CornerRadius,
    /// Colour of the first solid fill (a node without one cannot animate this).
    FillColor,
    /// Colour of the first solid stroke paint.
    StrokeColor,
    StrokeWeight,
    /// Horizontal offset of the first drop shadow (a node without one cannot animate this).
    ShadowX,
    /// Vertical offset of the first drop shadow.
    ShadowY,
    /// Blur radius of the first drop shadow.
    ShadowBlur,
    /// Colour (with alpha) of the first drop shadow.
    ShadowColor,
    /// Radius of the first layer blur.
    BlurRadius,
}

/// The shape of an [`AnimValue`] a property takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Scalar,
    Vec2,
    Color,
}

impl AnimProperty {
    /// Every property in inspector order (Transform, Layout, Appearance, Fill, Stroke, Effects).
    pub const ALL: [AnimProperty; 15] = [
        AnimProperty::Position,
        AnimProperty::Scale,
        AnimProperty::Rotation,
        AnimProperty::Width,
        AnimProperty::Height,
        AnimProperty::Opacity,
        AnimProperty::CornerRadius,
        AnimProperty::FillColor,
        AnimProperty::StrokeColor,
        AnimProperty::StrokeWeight,
        AnimProperty::ShadowX,
        AnimProperty::ShadowY,
        AnimProperty::ShadowBlur,
        AnimProperty::ShadowColor,
        AnimProperty::BlurRadius,
    ];

    /// Label of the timeline's property row.
    pub fn name(&self) -> &'static str {
        match self {
            AnimProperty::Position => "Position",
            AnimProperty::Scale => "Scale",
            AnimProperty::Rotation => "Rotation",
            AnimProperty::Width => "Width",
            AnimProperty::Height => "Height",
            AnimProperty::Opacity => "Opacity",
            AnimProperty::CornerRadius => "Corner radius",
            AnimProperty::FillColor => "Fill",
            AnimProperty::StrokeColor => "Stroke",
            AnimProperty::StrokeWeight => "Stroke weight",
            AnimProperty::ShadowX => "Shadow X",
            AnimProperty::ShadowY => "Shadow Y",
            AnimProperty::ShadowBlur => "Shadow blur",
            AnimProperty::ShadowColor => "Shadow color",
            AnimProperty::BlurRadius => "Layer blur",
        }
    }

    pub fn kind(&self) -> ValueKind {
        match self {
            AnimProperty::Position | AnimProperty::Scale => ValueKind::Vec2,
            AnimProperty::FillColor | AnimProperty::StrokeColor | AnimProperty::ShadowColor => ValueKind::Color,
            _ => ValueKind::Scalar,
        }
    }

    /// Multiply the model value by this to get the number shown in the UI (100 for opacity and
    /// scale which display as percentages, else 1).
    pub fn display_factor(&self) -> f64 {
        match self {
            AnimProperty::Opacity | AnimProperty::Scale => 100.0,
            _ => 1.0,
        }
    }

    /// Unit suffix of the displayed value ("px", "%", "°"; empty for colours).
    pub fn unit(&self) -> &'static str {
        match self {
            AnimProperty::Opacity | AnimProperty::Scale => "%",
            AnimProperty::Rotation => "°",
            AnimProperty::FillColor | AnimProperty::StrokeColor | AnimProperty::ShadowColor => "",
            _ => "px",
        }
    }

    /// Whether `v` has the right shape for this property.
    pub fn accepts(&self, v: &AnimValue) -> bool {
        v.kind() == self.kind()
    }

    /// Neutral value used when a property has no base (e.g. before a first keyframe is read).
    pub fn neutral(&self) -> AnimValue {
        match self.kind() {
            ValueKind::Scalar => AnimValue::Scalar(if matches!(self, AnimProperty::Opacity) { 1.0 } else { 0.0 }),
            ValueKind::Vec2 => AnimValue::Vec2(if matches!(self, AnimProperty::Scale) { Vec2::new(1.0, 1.0) } else { Vec2::ZERO }),
            ValueKind::Color => AnimValue::Color(Color::BLACK),
        }
    }
}

/// A typed animated value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum AnimValue {
    Scalar(f64),
    Vec2(Vec2),
    Color(Color),
}

impl AnimValue {
    pub fn kind(&self) -> ValueKind {
        match self {
            AnimValue::Scalar(_) => ValueKind::Scalar,
            AnimValue::Vec2(_) => ValueKind::Vec2,
            AnimValue::Color(_) => ValueKind::Color,
        }
    }

    pub fn scalar(&self) -> Option<f64> {
        match self {
            AnimValue::Scalar(v) => Some(*v),
            _ => None,
        }
    }

    pub fn vec2(&self) -> Option<Vec2> {
        match self {
            AnimValue::Vec2(v) => Some(*v),
            _ => None,
        }
    }

    pub fn color(&self) -> Option<Color> {
        match self {
            AnimValue::Color(v) => Some(*v),
            _ => None,
        }
    }

    pub fn is_finite(&self) -> bool {
        match self {
            AnimValue::Scalar(v) => v.is_finite(),
            AnimValue::Vec2(v) => v.x.is_finite() && v.y.is_finite(),
            AnimValue::Color(c) => c.r.is_finite() && c.g.is_finite() && c.b.is_finite() && c.a.is_finite(),
        }
    }

    /// Interpolates `a` to `b` at progress `t`. `t` is not clamped, so eased overshoot works;
    /// colour channels are clamped to 0..=1 afterwards. Values of different kinds do not mix: `a`
    /// is returned.
    pub fn lerp(a: &AnimValue, b: &AnimValue, t: f64) -> AnimValue {
        match (a, b) {
            (AnimValue::Scalar(x), AnimValue::Scalar(y)) => AnimValue::Scalar(lerp_f64(*x, *y, t)),
            (AnimValue::Vec2(x), AnimValue::Vec2(y)) => AnimValue::Vec2(lerp_vec2(*x, *y, t)),
            (AnimValue::Color(x), AnimValue::Color(y)) => AnimValue::Color(lerp_color(*x, *y, t as f32)),
            _ => *a,
        }
    }
}

pub fn lerp_f64(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

pub fn lerp_vec2(a: Vec2, b: Vec2, t: f64) -> Vec2 {
    Vec2::new(lerp_f64(a.x, b.x, t), lerp_f64(a.y, b.y, t))
}

/// Colour interpolation: **premultiplied alpha in sRGB-encoded space**.
pub fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let alpha = lerp_f32(a.a, b.a, t).clamp(0.0, 1.0);
    let ch = |x: f32, y: f32| {
        let p = lerp_f32(x * a.a, y * b.a, t);
        if alpha > 1e-6 { p / alpha } else { lerp_f32(x, y, t) }
    };
    Color { r: ch(a.r, b.r).clamp(0.0, 1.0), g: ch(a.g, b.g).clamp(0.0, 1.0), b: ch(a.b, b.b).clamp(0.0, 1.0), a: alpha }
}

fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_and_vec_lerp() {
        let m = AnimValue::lerp(&AnimValue::Scalar(10.0), &AnimValue::Scalar(20.0), 0.25);
        assert_eq!(m, AnimValue::Scalar(12.5));
        let v = AnimValue::lerp(&AnimValue::Vec2(Vec2::new(0.0, 10.0)), &AnimValue::Vec2(Vec2::new(10.0, 0.0)), 0.5);
        assert_eq!(v, AnimValue::Vec2(Vec2::new(5.0, 5.0)));
        // Overshoot is allowed for scalars.
        assert_eq!(AnimValue::lerp(&AnimValue::Scalar(0.0), &AnimValue::Scalar(1.0), 1.2), AnimValue::Scalar(1.2));
        // Mismatched kinds keep the start value.
        assert_eq!(AnimValue::lerp(&AnimValue::Scalar(1.0), &AnimValue::Vec2(Vec2::ZERO), 0.5), AnimValue::Scalar(1.0));
    }

    #[test]
    fn color_midpoint_is_hex_average() {
        let m = lerp_color(Color::BLACK, Color::WHITE, 0.5);
        assert!((m.r - 0.5).abs() < 1e-5 && (m.a - 1.0).abs() < 1e-6);
        assert_eq!(lerp_color(Color::BLACK, Color::WHITE, 0.0), Color::BLACK);
        assert_eq!(lerp_color(Color::BLACK, Color::WHITE, 1.0), Color::WHITE);
    }

    #[test]
    fn color_fade_to_transparent_keeps_hue() {
        let red = Color::from_rgb8_const(255, 0, 0);
        let m = lerp_color(red, Color::TRANSPARENT, 0.5);
        assert!((m.r - 1.0).abs() < 1e-5 && m.g.abs() < 1e-5, "{m:?}");
        assert!((m.a - 0.5).abs() < 1e-5);
        let z = lerp_color(Color::TRANSPARENT, Color::TRANSPARENT, 0.5);
        assert!(z.r.is_finite() && z.a == 0.0);
    }

    #[test]
    fn property_metadata() {
        for p in AnimProperty::ALL {
            assert!(p.accepts(&p.neutral()), "{p:?}");
            assert!(!p.name().is_empty());
        }
        assert_eq!(AnimProperty::Opacity.display_factor(), 100.0);
        assert_eq!(AnimProperty::Rotation.unit(), "°");
        assert_eq!(AnimProperty::Scale.neutral(), AnimValue::Vec2(Vec2::new(1.0, 1.0)));
    }
}
