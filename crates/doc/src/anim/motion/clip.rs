//! Animation clips.
//!
//! A clip animates one *kind* of change (position, scale, rotation, size, opacity) relative to
//! whatever the node would otherwise look like at that time.

use super::easing::Ease;
use super::value::{AnimProperty, lerp_vec2};
use figcraft_geom::Vec2;
use serde::{Deserialize, Serialize};

/// What a clip changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClipKind {
    Position,
    Scale,
    Rotation,
    /// Width and height.
    Size,
    Opacity,
}

impl ClipKind {
    pub const ALL: [ClipKind; 5] = [ClipKind::Position, ClipKind::Scale, ClipKind::Rotation, ClipKind::Size, ClipKind::Opacity];

    pub fn name(&self) -> &'static str {
        match self {
            ClipKind::Position => "Position",
            ClipKind::Scale => "Scale",
            ClipKind::Rotation => "Rotation",
            ClipKind::Size => "Size",
            ClipKind::Opacity => "Opacity",
        }
    }

    /// Animatable properties this kind of clip drives.
    pub fn properties(&self) -> &'static [AnimProperty] {
        match self {
            ClipKind::Position => &[AnimProperty::Position],
            ClipKind::Scale => &[AnimProperty::Scale],
            ClipKind::Rotation => &[AnimProperty::Rotation],
            ClipKind::Size => &[AnimProperty::Width, AnimProperty::Height],
            ClipKind::Opacity => &[AnimProperty::Opacity],
        }
    }

    /// Value that leaves the node unchanged (`.x` only for scalar kinds).
    pub fn neutral(&self) -> Vec2 {
        match self {
            ClipKind::Position | ClipKind::Rotation => Vec2::ZERO,
            ClipKind::Scale | ClipKind::Size => Vec2::new(1.0, 1.0),
            ClipKind::Opacity => Vec2::new(1.0, 0.0),
        }
    }

    /// Directions the inspector offers for this kind.
    pub fn directions(&self) -> &'static [ClipDirection] {
        use ClipDirection::*;
        match self {
            ClipKind::Position => &[Top, Bottom, Left, Right, TopLeft, TopRight, BottomLeft, BottomRight],
            ClipKind::Rotation => &[Clockwise, CounterClockwise],
            ClipKind::Scale | ClipKind::Size => &[Both, Horizontal, Vertical],
            ClipKind::Opacity => &[],
        }
    }

    pub fn default_direction(&self) -> ClipDirection {
        match self {
            ClipKind::Position => ClipDirection::Bottom,
            ClipKind::Rotation => ClipDirection::Clockwise,
            _ => ClipDirection::Both,
        }
    }

    /// What `amount` means: distance in px (Position), start/end factor (Scale, Size), degrees
    /// (Rotation), start/end opacity (Opacity).
    pub fn default_amount(&self) -> f32 {
        match self {
            ClipKind::Position => 100.0,
            ClipKind::Scale | ClipKind::Size => 0.5,
            ClipKind::Rotation => 90.0,
            ClipKind::Opacity => 0.0,
        }
    }
}

/// The "Type" dropdown of a clip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClipMode {
    /// Slide in: starts away from the node's normal state and settles into it.
    #[default]
    In,
    /// Slide out: starts at the normal state and moves away.
    Out,
    /// Free `from` / `to` values.
    Custom,
}

impl ClipMode {
    /// Label for a kind, e.g. "Slide in" for Position, "Scale in" for Scale.
    pub fn label(&self, kind: ClipKind) -> String {
        let verb = match kind {
            ClipKind::Position => "Slide",
            ClipKind::Scale => "Scale",
            ClipKind::Rotation => "Rotate",
            ClipKind::Size => "Resize",
            ClipKind::Opacity => "Fade",
        };
        match self {
            ClipMode::In => format!("{verb} in"),
            ClipMode::Out => format!("{verb} out"),
            ClipMode::Custom => "Custom".to_string(),
        }
    }
}

/// Where a clip comes from / goes to, or how it turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClipDirection {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Clockwise,
    CounterClockwise,
    /// Scale and size: both axes.
    Both,
    /// Scale and size: width only.
    Horizontal,
    /// Scale and size: height only.
    Vertical,
}

impl ClipDirection {
    /// Unit vector in y-down parent space for the position directions.
    pub fn vector(&self) -> Vec2 {
        const D: f64 = std::f64::consts::FRAC_1_SQRT_2;
        match self {
            ClipDirection::Top => Vec2::new(0.0, -1.0),
            ClipDirection::Bottom => Vec2::new(0.0, 1.0),
            ClipDirection::Left => Vec2::new(-1.0, 0.0),
            ClipDirection::Right => Vec2::new(1.0, 0.0),
            ClipDirection::TopLeft => Vec2::new(-D, -D),
            ClipDirection::TopRight => Vec2::new(D, -D),
            ClipDirection::BottomLeft => Vec2::new(-D, D),
            ClipDirection::BottomRight => Vec2::new(D, D),
            _ => Vec2::ZERO,
        }
    }

    /// Axis mask for scale / size: which axes the amount applies to.
    fn axes(&self) -> Vec2 {
        match self {
            ClipDirection::Horizontal => Vec2::new(1.0, 0.0),
            ClipDirection::Vertical => Vec2::new(0.0, 1.0),
            _ => Vec2::new(1.0, 1.0),
        }
    }

    /// Dropdown label: "From bottom" / "To bottom", "Clockwise", "Horizontal".
    pub fn label(&self, mode: ClipMode) -> String {
        let lead = if mode == ClipMode::Out { "To" } else { "From" };
        let side = |s: &str| format!("{lead} {s}");
        match self {
            ClipDirection::Top => side("top"),
            ClipDirection::Bottom => side("bottom"),
            ClipDirection::Left => side("left"),
            ClipDirection::Right => side("right"),
            ClipDirection::TopLeft => side("top left"),
            ClipDirection::TopRight => side("top right"),
            ClipDirection::BottomLeft => side("bottom left"),
            ClipDirection::BottomRight => side("bottom right"),
            ClipDirection::Clockwise => "Clockwise".into(),
            ClipDirection::CounterClockwise => "Counter-clockwise".into(),
            ClipDirection::Both => "Both".into(),
            ClipDirection::Horizontal => "Horizontal".into(),
            ClipDirection::Vertical => "Vertical".into(),
        }
    }
}

/// How often a clip's cycle runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipRepeat {
    #[default]
    Once,
    /// Run this many cycles (1 = once).
    Times(u32),
    /// Keep cycling until the end of the timeline.
    Infinite,
}

fn yes() -> bool {
    true
}
fn is_yes(v: &bool) -> bool {
    *v
}
fn is_no(v: &bool) -> bool {
    !*v
}
fn is_zero(v: &Vec2) -> bool {
    v.x == 0.0 && v.y == 0.0
}
fn is_once(r: &ClipRepeat) -> bool {
    *r == ClipRepeat::Once
}
fn default_duration() -> u32 {
    500
}

/// One clip on a node's timeline (a "Position", "Scale", ... bar).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// Unique among the node's clips; stable across edits so the UI can keep a selection.
    pub id: u32,
    pub kind: ClipKind,
    /// Slide in / Slide out / Custom.
    #[serde(default)]
    pub mode: ClipMode,
    #[serde(default = "default_direction")]
    pub direction: ClipDirection,
    /// Distance (px), scale or size factor, degrees, or opacity.
    #[serde(default)]
    pub amount: f32,
    /// Custom mode start value (`.x` only for rotation and opacity).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub from: Vec2,
    /// Custom mode end value.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub to: Vec2,
    /// When the clip starts, in ms on the timeline ("Delay").
    #[serde(default)]
    pub delay_ms: u32,
    /// Length of one cycle in ms.
    #[serde(default = "default_duration")]
    pub duration_ms: u32,
    #[serde(default)]
    pub easing: Ease,
    #[serde(default, skip_serializing_if = "is_once")]
    pub repeat: ClipRepeat,
    /// Odd cycles play backwards, so repeating clips go there and back (Pulse, Float).
    #[serde(default, skip_serializing_if = "is_no")]
    pub yoyo: bool,
    /// The eye toggle: hidden clips contribute nothing.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub visible: bool,
}

fn default_direction() -> ClipDirection {
    ClipDirection::Bottom
}

impl Clip {
    /// A clip of `kind` with the kind's default direction/amount, 500ms, ease out, at t = 0.
    pub fn new(kind: ClipKind, mode: ClipMode) -> Self {
        Self {
            id: 0,
            kind,
            mode,
            direction: kind.default_direction(),
            amount: kind.default_amount(),
            from: kind.neutral(),
            to: kind.neutral(),
            delay_ms: 0,
            duration_ms: 500,
            easing: Ease::EaseOut,
            repeat: ClipRepeat::Once,
            yoyo: false,
            visible: true,
        }
    }

    pub fn dir(mut self, d: ClipDirection) -> Self {
        self.direction = d;
        self
    }
    pub fn amount(mut self, a: f32) -> Self {
        self.amount = a;
        self
    }
    pub fn custom(mut self, from: Vec2, to: Vec2) -> Self {
        self.mode = ClipMode::Custom;
        self.from = from;
        self.to = to;
        self
    }
    pub fn delay(mut self, ms: u32) -> Self {
        self.delay_ms = ms;
        self
    }
    pub fn dur(mut self, ms: u32) -> Self {
        self.duration_ms = ms;
        self
    }
    pub fn ease(mut self, e: Ease) -> Self {
        self.easing = e;
        self
    }
    pub fn repeat(mut self, r: ClipRepeat) -> Self {
        self.repeat = r;
        self
    }
    pub fn yoyo(mut self) -> Self {
        self.yoyo = true;
        self
    }

    /// Clamps fields to usable values after loading or UI edits.
    pub fn sanitize(&mut self) {
        self.duration_ms = self.duration_ms.max(1);
        if !self.amount.is_finite() {
            self.amount = self.kind.default_amount();
        }
        for v in [&mut self.from, &mut self.to] {
            if !v.x.is_finite() || !v.y.is_finite() {
                *v = self.kind.neutral();
            }
        }
        self.easing = self.easing.sanitized();
        if let ClipRepeat::Times(0) = self.repeat {
            self.repeat = ClipRepeat::Once;
        }
    }

    /// Start and end value of one cycle in the kind's own value space.
    pub fn endpoints(&self) -> (Vec2, Vec2) {
        let n = self.kind.neutral();
        if self.mode == ClipMode::Custom {
            return (self.from, self.to);
        }
        if self.kind == ClipKind::Rotation {
            let amount = self.amount as f64;
            let turn = if self.direction == ClipDirection::CounterClockwise { amount } else { -amount };
            return if self.mode == ClipMode::In { (Vec2::new(-turn, 0.0), n) } else { (n, Vec2::new(turn, 0.0)) };
        }
        let amount = self.amount as f64;
        let away = match self.kind {
            ClipKind::Position => {
                let v = self.direction.vector();
                Vec2::new(v.x * amount, v.y * amount)
            }
            ClipKind::Scale | ClipKind::Size => {
                let ax = self.direction.axes();
                Vec2::new(1.0 + (amount - 1.0) * ax.x, 1.0 + (amount - 1.0) * ax.y)
            }
            _ => Vec2::new(amount, 0.0),
        };
        if self.mode == ClipMode::In { (away, n) } else { (n, away) }
    }

    /// Total length in ms of all cycles, `None` for an infinite clip.
    pub fn total_ms(&self) -> Option<u32> {
        let d = self.duration_ms.max(1);
        match self.repeat {
            ClipRepeat::Once => Some(d),
            ClipRepeat::Times(n) => Some(d.saturating_mul(n.max(1))),
            ClipRepeat::Infinite => None,
        }
    }

    /// End of the clip on the timeline; infinite clips end with the timeline.
    pub fn end_ms(&self, timeline_duration_ms: u32) -> u32 {
        match self.total_ms() {
            Some(total) => self.delay_ms.saturating_add(total),
            None => timeline_duration_ms.max(self.delay_ms.saturating_add(self.duration_ms.max(1))),
        }
    }

    /// Eased progress through the clip at timeline time `t_ms`.
    pub fn progress_at(&self, t_ms: f64) -> f32 {
        let local = t_ms - self.delay_ms as f64;
        if local.is_nan() || local <= 0.0 {
            return 0.0;
        }
        let n = local / self.duration_ms.max(1) as f64;
        let cycles = match self.repeat {
            ClipRepeat::Once => Some(1u32),
            ClipRepeat::Times(k) => Some(k.max(1)),
            ClipRepeat::Infinite => None,
        };
        if let Some(k) = cycles
            && n >= k as f64
        {
            let reversed = self.yoyo && (k - 1) % 2 == 1;
            return if reversed { 0.0 } else { 1.0 };
        }
        let idx = n.floor();
        let frac = (n - idx) as f32;
        let reversed = self.yoyo && (idx as u64) % 2 == 1;
        self.easing.ease(if reversed { 1.0 - frac } else { frac })
    }

    /// The clip's value (kind's own space) at timeline time `t_ms`.
    pub fn value_at(&self, t_ms: f64) -> Vec2 {
        let (a, b) = self.endpoints();
        lerp_vec2(a, b, self.progress_at(t_ms) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn slide_in_from_bottom_offsets_y() {
        let c = Clip::new(ClipKind::Position, ClipMode::In).dir(ClipDirection::Bottom).amount(116.0).delay(997).dur(300).ease(Ease::Linear);
        assert_eq!(c.endpoints(), (Vec2::new(0.0, 116.0), Vec2::ZERO));
        assert_eq!(c.value_at(0.0), Vec2::new(0.0, 116.0));
        assert!(close(c.value_at(997.0 + 150.0).y, 58.0));
        assert_eq!(c.value_at(1297.0), Vec2::ZERO);
    }

    #[test]
    fn slide_out_ends_away() {
        let c = Clip::new(ClipKind::Position, ClipMode::Out).dir(ClipDirection::Left).amount(80.0).dur(200).ease(Ease::Linear);
        assert_eq!(c.value_at(0.0), Vec2::ZERO);
        assert_eq!(c.value_at(1000.0), Vec2::new(-80.0, 0.0));
    }

    #[test]
    fn scale_opacity_rotation_endpoints() {
        let s = Clip::new(ClipKind::Scale, ClipMode::In).amount(0.5);
        assert_eq!(s.endpoints(), (Vec2::new(0.5, 0.5), Vec2::new(1.0, 1.0)));
        let o = Clip::new(ClipKind::Opacity, ClipMode::In).amount(0.0);
        assert_eq!(o.endpoints(), (Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0)));
        let r = Clip::new(ClipKind::Rotation, ClipMode::In).amount(90.0).dir(ClipDirection::Clockwise);
        assert_eq!(r.endpoints(), (Vec2::new(90.0, 0.0), Vec2::ZERO));
        let r = Clip::new(ClipKind::Rotation, ClipMode::Out).amount(90.0).dir(ClipDirection::Clockwise);
        assert_eq!(r.endpoints(), (Vec2::ZERO, Vec2::new(-90.0, 0.0)));
    }

    #[test]
    fn repeat_and_yoyo() {
        let c = Clip::new(ClipKind::Scale, ClipMode::Custom)
            .custom(Vec2::new(1.0, 1.0), Vec2::new(2.0, 2.0))
            .ease(Ease::Linear)
            .dur(100)
            .repeat(ClipRepeat::Infinite)
            .yoyo();
        assert!(close(c.value_at(50.0).x, 1.5));
        assert!(close(c.value_at(150.0).x, 1.5));
        assert!(close(c.value_at(110.0).x, 1.9));
    }

    #[test]
    fn span_of_clips() {
        let c = Clip::new(ClipKind::Opacity, ClipMode::In).delay(200).dur(300);
        assert_eq!(c.end_ms(2000), 500);
        assert_eq!(c.clone().repeat(ClipRepeat::Times(3)).end_ms(2000), 1100);
        assert_eq!(c.clone().repeat(ClipRepeat::Infinite).end_ms(2000), 2000);
    }

    #[test]
    fn labels() {
        assert_eq!(ClipMode::In.label(ClipKind::Position), "Slide in");
        assert_eq!(ClipDirection::Bottom.label(ClipMode::In), "From bottom");
        assert_eq!(ClipDirection::Bottom.label(ClipMode::Out), "To bottom");
    }
}
