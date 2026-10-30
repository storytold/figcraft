//! Easing curves for Motion keyframes and clips.
//!
//! An [`Ease`] maps normalised segment time `t` in `0..=1` to normalised progress (0 at the start
//! value, 1 at the end value). Back and spring curves may leave `0..=1` in between; every curve is
//! exactly 0 at `t = 0` and 1 at `t = 1`.

use serde::{Deserialize, Serialize};

/// Elasticity of the default custom spring.
pub const DEFAULT_ELASTICITY: f32 = 0.25;

/// Usable elasticity: 0 is (near) critically damped, 1.2 is as bouncy as the mapping allows.
pub const ELASTICITY_RANGE: (f32, f32) = (0.0, 1.2);

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

/// Settle threshold: a spring counts as settled once it is within 0.1% of its target.
pub const SPRING_SETTLE_EPS: f32 = 0.001;

/// `K = −ln(SPRING_SETTLE_EPS)` = 6.9078, the constant of the elasticity → frequency mapping.
pub const SPRING_SETTLE_K: f32 = 6.907_755;

/// Lowest damping ratio the mapping reaches (elasticity 1.2 and above).
const ZETA_MIN: f32 = 0.03;
/// Highest damping ratio the mapping reaches (elasticity 0), just short of critical damping.
const ZETA_MAX: f32 = 0.9999;

/// Damping ratio of an elasticity: `ζ = clamp(1 − e, 0.03, 0.9999)`.
pub fn spring_damping_ratio(elasticity: f32) -> f32 {
    let e = if elasticity.is_finite() { elasticity.clamp(ELASTICITY_RANGE.0, ELASTICITY_RANGE.1) } else { DEFAULT_ELASTICITY };
    (1.0 - e).clamp(ZETA_MIN, ZETA_MAX)
}

/// How the settle envelope bounds the oscillation, and so how `ω₀` is recovered from it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpringAlgorithm {
    /// The triangle inequality's loose bound on `|cos + B sin|`, i.e. `1 + B` with
    /// `B = ζ/√(1−ζ²)`. Never underestimates the envelope, so the settled frequency comes out
    /// a little high and the curve finishes early.
    #[default]
    Envelope,
    /// The auxiliary-angle amplitude `1/√(1−ζ²)` — the peak the oscillation can actually reach —
    /// so `ω₀` is the exact frequency of a spring that has decayed to the settle epsilon at the
    /// end of the segment.
    Tight,
}

/// Natural frequency of an elasticity under `algorithm`, in units of 1 / settle duration:
/// `Envelope` → `ω₀ = (K + ln(1 + ζ/√(1−ζ²))) / ζ`; `Tight` → `ω₀ = (K − ½ln(1−ζ²)) / ζ`.
pub fn spring_angular_frequency(elasticity: f32, algorithm: SpringAlgorithm) -> f32 {
    let z = spring_damping_ratio(elasticity);
    let root = (1.0 - z * z).max(1e-6).sqrt();
    let ln = match algorithm {
        SpringAlgorithm::Envelope => (1.0 + z / root).ln(),
        // ½·ln(1/(1−ζ²)) = −½·ln(1−ζ²).
        SpringAlgorithm::Tight => -0.5 * (1.0 - z * z).max(1e-6).ln(),
    };
    (SPRING_SETTLE_K + ln) / z
}

/// `(stiffness, damping)` of an elasticity at `mass`: `k = mass·ω₀²`, `c = 2·mass·ζ·ω₀`.
pub fn spring_stiffness_damping(elasticity: f32, mass: f32, algorithm: SpringAlgorithm) -> (f32, f32) {
    let m = if mass.is_finite() && mass > 0.0 { mass } else { 1.0 };
    let (z, w0) = (spring_damping_ratio(elasticity), spring_angular_frequency(elasticity, algorithm));
    (m * w0 * w0, 2.0 * m * z * w0)
}

/// Height of the first overshoot peak, as a fraction of the travel (0 without overshoot).
pub fn spring_overshoot(elasticity: f32) -> f32 {
    let z = spring_damping_ratio(elasticity);
    if z >= 1.0 {
        return 0.0;
    }
    (-std::f32::consts::PI * z / (1.0 - z * z).sqrt()).exp()
}

/// Spring response: progress at `t`, the fraction of the settle duration. The curve is the
/// closed-form step response of the mass/stiffness/damping triple the mapping derives from `e`.
pub fn spring_unit_response(elasticity: f32, algorithm: SpringAlgorithm, t: f32) -> f32 {
    if !t.is_finite() {
        return 0.0;
    }
    let t = t.clamp(0.0, 1.0);
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let (z, w0) = (spring_damping_ratio(elasticity), spring_angular_frequency(elasticity, algorithm));
    let y = spring_step(z, w0, t);
    // The response ends a hair short of 1 (that is what "settled to 0.1%" means); straighten the
    // residual so the segment lands exactly on the next keyframe.
    let end = spring_step(z, w0, 1.0);
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
    /// Duration-based spring: runs for exactly the segment; `elasticity` in
    /// [`ELASTICITY_RANGE`], the single knob the editor exposes.
    CustomSpring {
        #[serde(alias = "bounce")]
        elasticity: f32,
        /// Which settle envelope recovers the frequency (the loose bound, or the tight one).
        #[serde(default)]
        algorithm: SpringAlgorithm,
        /// Mass the stiffness and damping are expressed against; 1 by default, the value the
        /// elasticity mapping is calibrated at.
        #[serde(default = "unit_mass")]
        mass: f32,
    },
}

/// Default mass of a spring: the mapping's calibration point.
pub fn unit_mass() -> f32 {
    1.0
}

/// Which tab of the easing editor a curve belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EaseFamily {
    Hold,
    Curve,
    Spring,
}

impl Ease {
    /// Presets in dropdown order (no custom entries).
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

    pub fn spring(elasticity: f32) -> Self {
        Ease::spring_with(elasticity, SpringAlgorithm::Envelope, unit_mass())
    }

    /// A spring with the settle envelope and mass spelled out.
    pub fn spring_with(elasticity: f32, algorithm: SpringAlgorithm, mass: f32) -> Self {
        let e = if elasticity.is_finite() { elasticity.clamp(ELASTICITY_RANGE.0, ELASTICITY_RANGE.1) } else { DEFAULT_ELASTICITY };
        let m = if mass.is_finite() && mass > 0.0 { mass } else { unit_mass() };
        Ease::CustomSpring { elasticity: e, algorithm, mass: m }
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

    /// Elasticity of every spring (presets included); `None` for non-springs.
    pub fn elasticity(&self) -> Option<f32> {
        match self {
            Ease::SpringGentle => Some(0.25),
            Ease::SpringQuick => Some(0.42),
            Ease::SpringBouncy => Some(0.7),
            Ease::SpringSlow => Some(0.0),
            Ease::CustomSpring { elasticity, .. } => Some(*elasticity),
            _ => None,
        }
    }

    /// Settle envelope of every spring; `None` for non-springs.
    pub fn spring_algorithm(&self) -> Option<SpringAlgorithm> {
        match self {
            Ease::SpringGentle | Ease::SpringQuick | Ease::SpringBouncy | Ease::SpringSlow => Some(SpringAlgorithm::Envelope),
            Ease::CustomSpring { algorithm, .. } => Some(*algorithm),
            _ => None,
        }
    }

    /// Mass every spring's stiffness and damping are expressed against; `None` for non-springs.
    pub fn mass(&self) -> Option<f32> {
        match self {
            Ease::SpringGentle | Ease::SpringQuick | Ease::SpringBouncy | Ease::SpringSlow => Some(unit_mass()),
            Ease::CustomSpring { mass, .. } => Some(*mass),
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
        } else if let Some(b) = self.elasticity() {
            Ease::spring_with(b, self.spring_algorithm().unwrap_or_default(), self.mass().unwrap_or_else(unit_mass))
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
            Ease::CustomSpring { elasticity, algorithm, mass } => {
                let e = if elasticity.is_finite() { elasticity.clamp(ELASTICITY_RANGE.0, ELASTICITY_RANGE.1) } else { DEFAULT_ELASTICITY };
                let m = if mass.is_finite() && *mass > 0.0 { *mass } else { unit_mass() };
                Ease::CustomSpring { elasticity: e, algorithm: *algorithm, mass: m }
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
            e => match (e.control_points(), e.elasticity()) {
                (Some(c), _) => cubic_bezier(c, t),
                (None, Some(b)) => spring_unit_response(b, e.spring_algorithm().unwrap_or_default(), t),
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
    use super::{
        Ease, SPRING_SETTLE_EPS, SpringAlgorithm, spring_angular_frequency, spring_damping_ratio, spring_overshoot, spring_stiffness_damping,
    };

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn all_curves() -> Vec<Ease> {
        let mut v: Vec<Ease> = Ease::PRESETS.iter().map(|p| p.1).collect();
        v.extend([Ease::bezier(0.07, 0.97, 0.58, 1.0), Ease::spring(0.0), Ease::spring(0.25), Ease::spring(1.0), Ease::spring(1.2)]);
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
    fn spring_mapping() {
        // ζ = clamp(1 − e, .03, .9999); ω₀ = (K + ln(1 + ζ/√(1−ζ²)))/ζ; k = ω₀², c = 2ζω₀.
        assert!(close(spring_damping_ratio(0.6), 0.4));
        assert!((spring_angular_frequency(0.6, SpringAlgorithm::Envelope) - 18.175).abs() < 0.01);
        let (k, c) = spring_stiffness_damping(0.6, 1.0, SpringAlgorithm::Envelope);
        assert!((k - 330.3).abs() < 0.5, "{k}");
        assert!((c - 14.54).abs() < 0.02, "{c}");
        // The mass only rescales the physical pair; ζ and ω₀ (and so the curve) stay put.
        let (k2, c2) = spring_stiffness_damping(0.6, 2.0, SpringAlgorithm::Envelope);
        assert!((k2 - 2.0 * k).abs() < 0.5 && (c2 - 2.0 * c).abs() < 0.05);
        assert!((spring_overshoot(0.6) - 0.2538).abs() < 0.001);
        // The mapping clamps at both ends; a higher elasticity is both faster and bouncier.
        assert!(close(spring_damping_ratio(-2.0), 0.9999));
        assert!(close(spring_damping_ratio(5.0), 0.03));
        let env = SpringAlgorithm::Envelope;
        assert!(spring_angular_frequency(1.0, env) > spring_angular_frequency(0.2, env));
        assert!(spring_overshoot(1.0) > spring_overshoot(0.2));
        // The segment is the settle duration: the curve peaks at 1 + overshoot and ends on 1.
        for e in [0.0, 0.2, 0.5, 0.6, 0.8, 1.0] {
            let unit = Ease::spring(e);
            assert_eq!(unit.ease(1.0), 1.0);
            let peak = (1..=1000).map(|i| unit.ease(i as f32 / 1000.0)).fold(0.0f32, f32::max);
            assert!((peak - (1.0 + spring_overshoot(e))).abs() < 0.01, "{e}: {peak}");
        }
    }

    #[test]
    fn tight_envelope_is_the_exact_one() {
        // Loose bound `1 + B` and auxiliary-angle amplitude `1/√(1−ζ²)` both hit the settle
        // epsilon at t = 1, but the tight one is the peak the curve can actually reach, so it
        // recovers a lower — exact — frequency and uses the whole segment.
        for e in [0.0, 0.25, 0.6, 0.9] {
            let z = spring_damping_ratio(e);
            let root = (1.0 - z * z).sqrt();
            let a = spring_angular_frequency(e, SpringAlgorithm::Envelope);
            let b = spring_angular_frequency(e, SpringAlgorithm::Tight);
            assert!(b < a, "{e}: tight {b} should be below loose {a}");
            assert!(((-z * a).exp() * (1.0 + z / root) - SPRING_SETTLE_EPS).abs() < 2e-4, "{e} loose");
            assert!(((-z * b).exp() / root - SPRING_SETTLE_EPS).abs() < 2e-4, "{e} tight");
        }
        // e = 0.6 → ζ = 0.4: (K − ½ln 0.84)/0.4 = 17.487.
        assert!((spring_angular_frequency(0.6, SpringAlgorithm::Tight) - 17.487).abs() < 0.01);
        // The tight curve runs at the lower, exact frequency, so at the same instant it has
        // travelled a little less than the loose one.
        let (loose, exact) = (Ease::spring(0.6), Ease::spring_with(0.6, SpringAlgorithm::Tight, 1.0));
        assert!(exact.ease(0.4) < loose.ease(0.4));
        assert_eq!(exact.ease(0.0), 0.0);
        assert_eq!(exact.ease(1.0), 1.0);
    }

    #[test]
    fn springs_saved_as_bounce_still_load() {
        let old: Ease = serde_json::from_str(r#"{"CustomSpring":{"bounce":0.4}}"#).unwrap();
        assert_eq!(old, Ease::spring(0.4));
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
