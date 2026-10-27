//! Easing curves for Motion keyframes and clips.
//!
//! An [`Ease`] maps normalised segment time `t` in `0..=1` to normalised progress (0 at the start
//! value, 1 at the end value). Back and spring curves may leave `0..=1` in between; every curve is
//! exactly 0 at `t = 0` and 1 at `t = 1`.

use serde::{Deserialize, Serialize};

/// Bounce of the default custom spring (Figma's "Custom spring" starts at 0.25).
pub const DEFAULT_BOUNCE: f32 = 0.25;

/// y(t) of the unit cubic bezier through (0,0), (x1,y1), (x2,y2), (1,1), i.e. CSS `cubic-bezier`.
pub fn cubic_bezier(c: [f32; 4], t: f32) -> f32 {
    let [x1, y1, x2, y2] = c;
    let (x1, x2) = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
    if !t.is_finite() {
        return 0.0;
    }
    let t = t.clamp(0.0, 1.0);
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let sample = |p1: f32, p2: f32, s: f32| {
        let u = 1.0 - s;
        3.0 * u * u * s * p1 + 3.0 * u * s * s * p2 + s * s * s
    };
    let dx = |s: f32| {
        let u = 1.0 - s;
        3.0 * u * u * x1 + 6.0 * u * s * (x2 - x1) + 3.0 * s * s * (1.0 - x2)
    };
    let mut s = t;
    for _ in 0..8 {
        let err = sample(x1, x2, s) - t;
        if err.abs() < 1e-6 {
            return sample(y1, y2, s);
        }
        let d = dx(s);
        if d.abs() < 1e-6 {
            break;
        }
        s -= err / d;
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    s = t;
    for _ in 0..40 {
        let x = sample(x1, x2, s);
        if (x - t).abs() < 1e-6 {
            break;
        }
        if x < t {
            lo = s;
        } else {
            hi = s;
        }
        s = (lo + hi) / 2.0;
    }
    sample(y1, y2, s)
}

/// Step response of a damped spring (starts at 0, rests at 1) in normalised units.
pub fn spring_step(zeta: f32, w0: f32, tau: f32) -> f32 {
    if zeta < 1.0 - 1e-4 {
        let wd = w0 * (1.0 - zeta * zeta).sqrt();
        let a = zeta * w0;
        1.0 - (-a * tau).exp() * ((wd * tau).cos() + a / wd * (wd * tau).sin())
    } else if zeta <= 1.0 + 1e-4 {
        1.0 - (-w0 * tau).exp() * (1.0 + w0 * tau)
    } else {
        let s = (zeta * zeta - 1.0).sqrt();
        let (r1, r2) = (-w0 * (zeta - s), -w0 * (zeta + s));
        1.0 - (r2 * (r1 * tau).exp() - r1 * (r2 * tau).exp()) / (r2 - r1)
    }
}

fn spring_params(mass: f32, stiffness: f32, damping: f32) -> (f32, f32) {
    let (m, k, c) = (mass.max(0.01), stiffness.max(0.01), damping.max(0.0));
    (c / (2.0 * (k * m).sqrt()), (k / m).sqrt())
}

/// Seconds a mass/stiffness/damping spring needs to settle (envelope below 0.1%), capped at 4s.
pub fn spring_settle_secs(mass: f32, stiffness: f32, damping: f32) -> f32 {
    let (zeta, w0) = spring_params(mass, stiffness, damping);
    let rate = if zeta < 1.0 { zeta * w0 } else { w0 * (zeta - (zeta * zeta - 1.0).sqrt()) };
    if rate <= 1e-3 {
        return 4.0;
    }
    (6.9 / rate).clamp(0.1, 4.0)
}

/// Spring step response at time `tau` seconds (starts at 0, settles at 1).
pub fn spring_at(mass: f32, stiffness: f32, damping: f32, tau: f32) -> f32 {
    let (zeta, w0) = spring_params(mass, stiffness, damping);
    spring_step(zeta, w0, tau)
}

const SETTLE_NEPERS: f32 = 8.0;
const BOUNCE_RANGE: (f32, f32) = (-0.8, 0.9);

/// Damping ratio for a `bounce` value, following SwiftUI's `Spring(duration:bounce:)`.
pub fn bounce_to_damping_ratio(bounce: f32) -> f32 {
    let b = if bounce.is_finite() { bounce.clamp(BOUNCE_RANGE.0, BOUNCE_RANGE.1) } else { DEFAULT_BOUNCE };
    if b >= 0.0 { 1.0 - b } else { 1.0 / (1.0 + b) }
}

/// Duration-normalised spring: progress at normalised time `t` (0..=1).
pub fn spring_unit_response(bounce: f32, t: f32) -> f32 {
    if !t.is_finite() {
        return 0.0;
    }
    let t = t.clamp(0.0, 1.0);
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let zeta = bounce_to_damping_ratio(bounce);
    let rate = if zeta < 1.0 { zeta } else { zeta - (zeta * zeta - 1.0).sqrt() };
    let w0 = SETTLE_NEPERS / rate.max(1e-3);
    let y = spring_step(zeta, w0, t);
    let end = spring_step(zeta, w0, 1.0);
    y + (1.0 - end) * t
}

/// Easing of one keyframe segment or clip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Ease {
    /// Keep the start value until the next keyframe, then jump (stepped).
    Hold,
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    EaseInBack,
    EaseOutBack,
    EaseInOutBack,
    /// CSS-style control points `[x1, y1, x2, y2]`; x values are clamped to 0..=1.
    CustomBezier([f32; 4]),
    SpringGentle,
    SpringQuick,
    SpringBouncy,
    SpringSlow,
    /// Duration-based spring: runs for exactly the segment, `bounce` in about -0.8..=0.9.
    CustomSpring {
        bounce: f32,
    },
}

/// Which tab of the easing editor a curve belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EaseFamily {
    Hold,
    Curve,
    Spring,
}

impl Ease {
    /// Presets in the order of Figma's easing dropdown (no custom entries).
    pub const PRESETS: [(&'static str, Ease); 12] = [
        ("Hold", Ease::Hold),
        ("Linear", Ease::Linear),
        ("Ease in", Ease::EaseIn),
        ("Ease out", Ease::EaseOut),
        ("Ease in and out", Ease::EaseInOut),
        ("Ease in back", Ease::EaseInBack),
        ("Ease out back", Ease::EaseOutBack),
        ("Ease in and out back", Ease::EaseInOutBack),
        ("Gentle", Ease::SpringGentle),
        ("Quick", Ease::SpringQuick),
        ("Bouncy", Ease::SpringBouncy),
        ("Slow", Ease::SpringSlow),
    ];

    pub fn bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Ease::CustomBezier([x1.clamp(0.0, 1.0), y1, x2.clamp(0.0, 1.0), y2])
    }

    pub fn spring(bounce: f32) -> Self {
        Ease::CustomSpring { bounce }
    }

    pub fn is_linear(&self) -> bool {
        matches!(self, Ease::Linear)
    }

    pub fn family(&self) -> EaseFamily {
        match self {
            Ease::Hold => EaseFamily::Hold,
            Ease::SpringGentle | Ease::SpringQuick | Ease::SpringBouncy | Ease::SpringSlow | Ease::CustomSpring { .. } => EaseFamily::Spring,
            _ => EaseFamily::Curve,
        }
    }

    pub fn is_spring(&self) -> bool {
        self.family() == EaseFamily::Spring
    }

    /// Control points `[x1, y1, x2, y2]` of every bezier-based curve; `None` for Hold and springs.
    pub fn control_points(&self) -> Option<[f32; 4]> {
        Some(match self {
            Ease::Linear => [0.0, 0.0, 1.0, 1.0],
            Ease::EaseIn => [0.42, 0.0, 1.0, 1.0],
            Ease::EaseOut => [0.0, 0.0, 0.58, 1.0],
            Ease::EaseInOut => [0.42, 0.0, 0.58, 1.0],
            Ease::EaseInBack => [0.3, -0.05, 0.7, -0.5],
            Ease::EaseOutBack => [0.45, 1.45, 0.8, 1.0],
            Ease::EaseInOutBack => [0.7, -0.4, 0.4, 1.4],
            Ease::CustomBezier(c) => *c,
            _ => return None,
        })
    }

    /// Bounce of every spring (presets included); `None` for non-springs.
    pub fn bounce(&self) -> Option<f32> {
        match self {
            Ease::SpringGentle => Some(0.25),
            Ease::SpringQuick => Some(0.42),
            Ease::SpringBouncy => Some(0.7),
            Ease::SpringSlow => Some(-0.1),
            Ease::CustomSpring { bounce } => Some(*bounce),
            _ => None,
        }
    }

    /// Name shown in the preset dropdown.
    pub fn name(&self) -> &'static str {
        if let Some((n, _)) = Self::PRESETS.iter().find(|(_, p)| p == self) {
            return n;
        }
        match self {
            Ease::CustomBezier(_) => "Custom bezier",
            Ease::CustomSpring { .. } => "Custom spring",
            _ => "Custom",
        }
    }

    /// The same curve as an editable custom curve.
    pub fn to_custom(&self) -> Ease {
        if let Some(c) = self.control_points() {
            Ease::CustomBezier(c)
        } else if let Some(b) = self.bounce() {
            Ease::CustomSpring { bounce: b }
        } else {
            *self
        }
    }

    /// The curve with its parameters clamped to usable ranges.
    pub fn sanitized(&self) -> Ease {
        match self {
            Ease::CustomBezier(c) => {
                let f = |v: f32, d: f32| if v.is_finite() { v } else { d };
                Ease::bezier(f(c[0], 0.0), f(c[1], 0.0), f(c[2], 1.0), f(c[3], 1.0))
            }
            Ease::CustomSpring { bounce } => {
                Ease::CustomSpring { bounce: if bounce.is_finite() { bounce.clamp(BOUNCE_RANGE.0, BOUNCE_RANGE.1) } else { DEFAULT_BOUNCE } }
            }
            e => *e,
        }
    }

    /// Eased progress for linear segment progress `t` (clamped to 0..=1).
    pub fn ease(&self, t: f32) -> f32 {
        if !t.is_finite() {
            return 0.0;
        }
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Hold => {
                if t >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Ease::Linear => t,
            e => match (e.control_points(), e.bounce()) {
                (Some(c), _) => cubic_bezier(c, t),
                (None, Some(b)) => spring_unit_response(b, t),
                (None, None) => t,
            },
        }
    }

    /// `n + 1` points `[t, ease(t)]` for `t = 0, 1/n .. 1`, for drawing the graph.
    pub fn sample(&self, n: usize) -> Vec<[f32; 2]> {
        let n = n.max(1);
        let mut out: Vec<[f32; 2]> = (0..=n).map(|i| i as f32 / n as f32).map(|t| [t, self.ease(t)]).collect();
        if matches!(self, Ease::Hold) {
            let last = out.len() - 1;
            out.insert(last, [1.0, 0.0]);
        }
        out
    }

    /// Vertical extent `(min, max)` of the curve, always including 0 and 1.
    pub fn value_range(&self) -> (f32, f32) {
        self.sample(100).iter().fold((0.0f32, 1.0f32), |(lo, hi), p| (lo.min(p[1]), hi.max(p[1])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn all_curves() -> Vec<Ease> {
        let mut v: Vec<Ease> = Ease::PRESETS.iter().map(|p| p.1).collect();
        v.extend([Ease::bezier(0.07, 0.97, 0.58, 1.0), Ease::spring(0.0), Ease::spring(0.25), Ease::spring(0.9), Ease::spring(-0.5)]);
        v
    }

    #[test]
    fn endpoints_are_exact() {
        for e in all_curves() {
            assert_eq!(e.ease(0.0), 0.0, "{e:?} start");
            assert_eq!(e.ease(1.0), 1.0, "{e:?} end");
            assert_eq!(e.ease(-3.0), 0.0);
            assert_eq!(e.ease(7.0), 1.0);
            assert_eq!(e.ease(f32::NAN), 0.0);
        }
    }

    #[test]
    fn hold_steps() {
        assert_eq!(Ease::Hold.ease(0.5), 0.0);
        assert_eq!(Ease::Hold.ease(1.0), 1.0);
        let pts = Ease::Hold.sample(4);
        assert!(pts.contains(&[1.0, 0.0]) && pts.last() == Some(&[1.0, 1.0]));
    }

    #[test]
    fn standard_curves_have_expected_shape() {
        assert!(Ease::EaseIn.ease(0.5) < 0.5);
        assert!(Ease::EaseOut.ease(0.5) > 0.5);
        assert!(close(Ease::EaseInOut.ease(0.5), 0.5));
        assert!(close(Ease::Linear.ease(0.3), 0.3));
        assert!((Ease::EaseInOut.ease(0.25) - 0.129).abs() < 0.01);
    }

    #[test]
    fn presets_and_names() {
        assert_eq!(Ease::EaseOut.name(), "Ease out");
        assert_eq!(Ease::bezier(0.1, 0.2, 0.3, 0.4).name(), "Custom bezier");
        assert_eq!(Ease::spring(0.4).name(), "Custom spring");
        assert_eq!(Ease::SpringQuick.name(), "Quick");
        assert_eq!(Ease::default(), Ease::Linear);
    }

    #[test]
    fn serde_roundtrip() {
        for e in all_curves() {
            let back: Ease = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
            assert_eq!(back, e);
        }
    }
}
