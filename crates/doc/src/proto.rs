//! Prototype data: interactions (trigger → actions) with transitions, flows and overflow.

use crate::{CollectionId, ModeId, NodeId, Variables};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[path = "proto_expr.rs"]
mod expr;
pub use expr::*;
#[path = "proto_expr_parse.rs"]
mod expr_parse;
pub use expr_parse::*;

/// Deepest nesting of conditional blocks followed.
pub const MAX_ACTION_DEPTH: usize = 16;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
#[derive(Default)]
pub enum Trigger {
    #[default]
    OnClick,
    OnHover,
    OnPress,
    OnDrag,
    MouseEnter,
    MouseLeave,
    MouseDown,
    MouseUp,
    AfterDelay {
        ms: u32,
    },
    KeyPress {
        key: String,
    },
}

impl Trigger {
    pub fn label(&self) -> String {
        match self {
            Trigger::OnClick => "On click".into(),
            Trigger::OnHover => "While hovering".into(),
            Trigger::OnPress => "While pressing".into(),
            Trigger::OnDrag => "On drag".into(),
            Trigger::MouseEnter => "Mouse enter".into(),
            Trigger::MouseLeave => "Mouse leave".into(),
            Trigger::MouseDown => "Mouse down".into(),
            Trigger::MouseUp => "Mouse up".into(),
            Trigger::AfterDelay { ms } => format!("After delay {ms}ms"),
            // A controller button names itself ("Gamepad A").
            Trigger::KeyPress { key } if is_gamepad_button(key) => key.clone(),
            Trigger::KeyPress { key } => format!("Key/Gamepad {key}"),
        }
    }

    /// A key trigger fires for the key named `key` (`Space`, `K`, `ArrowLeft`) pressed with
    /// exactly the modifiers `mods`: its key is a combination such as `Shift+K`. A controller
    /// button is a key named from [`GAMEPAD_BUTTONS`], pressed without modifiers.
    pub fn fires_on_key(&self, key: &str, mods: KeyMods) -> bool {
        let Trigger::KeyPress { key: want } = self else { return false };
        let (m, k) = KeyMods::split(want);
        m == mods && (k.eq_ignore_ascii_case(key) || key_alias(k) == Some(key))
    }
}

/// The modifier keys of a key combination.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyMods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Command (macOS), the Windows key elsewhere.
    pub cmd: bool,
}

impl KeyMods {
    /// `key` with these modifiers, as a key trigger stores it: `Ctrl+Alt+Shift+Cmd+K`.
    pub fn combo(self, key: &str) -> String {
        let mut s = String::new();
        for (on, name) in [(self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift"), (self.cmd, "Cmd")] {
            if on {
                s.push_str(name);
                s.push('+');
            }
        }
        s + key
    }

    /// A key combination's modifiers and key (`Shift+K`, `Ctrl++`, `K`; other spellings of the
    /// modifiers too: Control, Option, Command, ⌘…).
    pub fn split(combo: &str) -> (KeyMods, &str) {
        let mut mods = KeyMods::default();
        let mut rest = combo.trim();
        while let Some((m, k)) = rest.split_once('+').filter(|(_, k)| !k.is_empty()) {
            match m.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "⌃" => mods.ctrl = true,
                "alt" | "option" | "opt" | "⌥" => mods.alt = true,
                "shift" | "⇧" => mods.shift = true,
                "cmd" | "command" | "meta" | "super" | "win" | "⌘" => mods.cmd = true,
                _ => break,
            }
            rest = k.trim();
        }
        (mods, rest)
    }
}

/// The controller buttons a key/gamepad trigger can name (as its key), in the order of the
/// standard gamepad layout's button indices (the browser Gamepad API's `standard` mapping): face
/// buttons by position (bottom, right, left, top), shoulders, triggers, the two centre buttons,
/// stick presses, the d-pad and the home button.
pub const GAMEPAD_BUTTONS: [&str; 17] = [
    "Gamepad A",
    "Gamepad B",
    "Gamepad X",
    "Gamepad Y",
    "Gamepad LB",
    "Gamepad RB",
    "Gamepad LT",
    "Gamepad RT",
    "Gamepad View",
    "Gamepad Menu",
    "Gamepad LS",
    "Gamepad RS",
    "Gamepad Up",
    "Gamepad Down",
    "Gamepad Left",
    "Gamepad Right",
    "Gamepad Home",
];

/// The button at standard-layout index `index`, as a key trigger names it.
pub fn gamepad_button(index: usize) -> Option<&'static str> {
    GAMEPAD_BUTTONS.get(index).copied()
}

/// Whether a key trigger's key is a controller button.
pub fn is_gamepad_button(key: &str) -> bool {
    GAMEPAD_BUTTONS.iter().any(|b| b.eq_ignore_ascii_case(key.trim()))
}

/// Whether any layer of `doc` has a key/gamepad trigger on a controller button (the player then
/// polls controllers).
pub fn uses_gamepad(doc: &crate::Document) -> bool {
    doc.nodes().any(|n| n.interactions.iter().any(|it| matches!(&it.trigger, Trigger::KeyPress { key } if is_gamepad_button(key))))
}

/// Alternative spellings people type for keys, as the keys' names.
pub fn key_alias(k: &str) -> Option<&'static str> {
    Some(match k.to_ascii_lowercase().as_str() {
        "space" | " " => "Space",
        "return" | "enter" => "Enter",
        "esc" | "escape" => "Escape",
        "left" | "arrowleft" | "←" => "ArrowLeft",
        "right" | "arrowright" | "→" => "ArrowRight",
        "up" | "arrowup" | "↑" => "ArrowUp",
        "down" | "arrowdown" | "↓" => "ArrowDown",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "delete" => "Delete",
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Easing {
    Linear,
    EaseIn,
    #[default]
    EaseOut,
    EaseInOut,
    EaseInBack,
    EaseOutBack,
    EaseInOutBack,
    Gentle,
    Quick,
    Bouncy,
    Slow,
    /// A CSS-style cubic bezier through (0,0), (x1,y1), (x2,y2), (1,1).
    #[serde(rename_all = "camelCase")]
    CustomBezier {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
    },
    /// A damped spring (mass, stiffness, damping), played over the interaction's duration.
    #[serde(rename_all = "camelCase")]
    CustomSpring {
        mass: f64,
        stiffness: f64,
        damping: f64,
    },
}

impl Easing {
    /// The starting point when switching to a custom curve.
    pub const CUSTOM_BEZIER: Easing = Easing::CustomBezier { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 };
    pub const CUSTOM_SPRING: Easing = Easing::CustomSpring { mass: 1.0, stiffness: 100.0, damping: 15.0 };

    pub const ALL: [Easing; 11] = [
        Easing::Linear,
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
        Easing::EaseInBack,
        Easing::EaseOutBack,
        Easing::EaseInOutBack,
        Easing::Gentle,
        Easing::Quick,
        Easing::Bouncy,
        Easing::Slow,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Easing::Linear => "Linear",
            Easing::EaseIn => "Ease in",
            Easing::EaseOut => "Ease out",
            Easing::EaseInOut => "Ease in and out",
            Easing::EaseInBack => "Ease in back",
            Easing::EaseOutBack => "Ease out back",
            Easing::EaseInOutBack => "Ease in and out back",
            Easing::Gentle => "Gentle",
            Easing::Quick => "Quick",
            Easing::Bouncy => "Bouncy",
            Easing::Slow => "Slow",
            Easing::CustomBezier { .. } => "Custom bezier",
            Easing::CustomSpring { .. } => "Custom spring",
        }
    }

    /// A spring's mass, stiffness and damping (None for the curves). The presets are physical
    /// springs too.
    pub fn spring(self) -> Option<(f64, f64, f64)> {
        match self {
            Easing::Gentle => Some((1.0, 100.0, 15.0)),
            Easing::Quick => Some((1.0, 300.0, 20.0)),
            Easing::Bouncy => Some((1.0, 600.0, 15.0)),
            Easing::Slow => Some((1.0, 80.0, 20.0)),
            Easing::CustomSpring { mass, stiffness, damping } => Some((mass, stiffness, damping)),
            _ => None,
        }
    }

    pub fn is_spring(self) -> bool {
        self.spring().is_some()
    }

    /// How long an animation with this easing plays (seconds) when its duration is set to `ms`:
    /// a spring runs until it settles, whatever the duration says.
    pub fn seconds(self, ms: u32) -> f64 {
        match self.spring() {
            Some((m, k, c)) => Spring::new(m, k, c).settle,
            None => f64::from(ms) / 1000.0,
        }
    }

    /// Evaluate the curve at `t` (0..1).
    pub fn eval(self, t: f64) -> f64 {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 1.0 };
        let back = 1.70158;
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t * t,
            Easing::EaseOut => 1.0 - (1.0 - t).powi(3),
            Easing::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Easing::EaseInBack => (back + 1.0) * t * t * t - back * t * t,
            Easing::EaseOutBack => 1.0 + (back + 1.0) * (t - 1.0).powi(3) + back * (t - 1.0).powi(2),
            Easing::EaseInOutBack => {
                let c2 = back * 1.525;
                if t < 0.5 {
                    ((2.0 * t).powi(2) * ((c2 + 1.0) * 2.0 * t - c2)) / 2.0
                } else {
                    ((2.0 * t - 2.0).powi(2) * ((c2 + 1.0) * (t * 2.0 - 2.0) + c2) + 2.0) / 2.0
                }
            }
            Easing::Gentle | Easing::Quick | Easing::Bouncy | Easing::Slow | Easing::CustomSpring { .. } => {
                let (m, k, c) = self.spring().unwrap_or((1.0, 100.0, 15.0));
                Spring::new(m, k, c).at(t)
            }
            Easing::CustomBezier { x1, y1, x2, y2 } => cubic_bezier(t, x1, y1, x2, y2),
        }
    }
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` at time `t` (x clamped to 0..1 so time stays monotonic).
fn cubic_bezier(t: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let fin = |v: f64, d: f64| if v.is_finite() { v } else { d };
    let (x1, x2) = (fin(x1, 0.42).clamp(0.0, 1.0), fin(x2, 0.58).clamp(0.0, 1.0));
    let (y1, y2) = (fin(y1, 0.0).clamp(-10.0, 10.0), fin(y2, 1.0).clamp(-10.0, 10.0));
    let b = |p1: f64, p2: f64, s: f64| {
        let u = 1.0 - s;
        3.0 * u * u * s * p1 + 3.0 * u * s * s * p2 + s * s * s
    };
    // x(s) is monotonic for x1, x2 in 0..1: bisect for s with x(s) = t.
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..48 {
        let mid = (lo + hi) / 2.0;
        if b(x1, x2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    b(y1, y2, (lo + hi) / 2.0)
}

/// A damped spring (unit mass-spring-damper released from 0 towards 1).
struct Spring {
    w0: f64,
    zeta: f64,
    /// Seconds until it stays within ~0.1% of the end (an undamped spring is cut at 10 s).
    settle: f64,
}

impl Spring {
    fn new(mass: f64, stiffness: f64, damping: f64) -> Self {
        let ok = |v: f64, lo: f64, d: f64| if v.is_finite() && v >= lo { v } else { d };
        let m = ok(mass, 0.01, 1.0).min(1e4);
        let k = ok(stiffness, 0.01, 100.0).min(1e6);
        let c = ok(damping, 0.0, 15.0).min(1e6);
        let w0 = (k / m).sqrt();
        let zeta = c / (2.0 * (k * m).sqrt());
        // The slowest mode's decay rate (overdamped: w0 (zeta - sqrt(zeta² - 1)), written stably);
        // e^-6.9 is 0.1%, and a critically or over-damped spring's larger lead factor needs ~9.2.
        let (rate, k_settle) = if zeta < 1.0 { (zeta * w0, 6.9) } else { (w0 / (zeta + (zeta * zeta - 1.0).sqrt()), 9.2) };
        let settle = if rate > 1e-6 { (k_settle / rate).clamp(0.05, 10.0) } else { 10.0 };
        Spring { w0, zeta, settle: if settle.is_finite() { settle } else { 10.0 } }
    }

    /// Its position at `t` (0..1) of its settling time.
    fn at(&self, t: f64) -> f64 {
        if t >= 1.0 {
            return 1.0;
        }
        let (w0, zeta) = (self.w0, self.zeta);
        let tau = t * self.settle;
        let x = if zeta < 1.0 {
            let wd = w0 * (1.0 - zeta * zeta).sqrt();
            1.0 - (-zeta * w0 * tau).exp() * ((wd * tau).cos() + zeta * w0 / wd * (wd * tau).sin())
        } else if (zeta - 1.0).abs() < 1e-9 {
            1.0 - (-w0 * tau).exp() * (1.0 + w0 * tau)
        } else {
            let s = (zeta * zeta - 1.0).sqrt();
            let (r1, r2) = (-w0 / (zeta + s), -w0 * (zeta + s));
            1.0 - (r2 * (r1 * tau).exp() - r1 * (r2 * tau).exp()) / (r2 - r1)
        };
        if x.is_finite() { x } else { 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
#[derive(Default)]
pub enum Transition {
    #[default]
    Instant,
    Dissolve,
    SmartAnimate,
    MoveIn {
        #[serde(default)]
        direction: Direction,
    },
    MoveOut {
        #[serde(default)]
        direction: Direction,
    },
    Push {
        #[serde(default)]
        direction: Direction,
    },
    SlideIn {
        #[serde(default)]
        direction: Direction,
    },
    SlideOut {
        #[serde(default)]
        direction: Direction,
    },
}

impl Transition {
    /// The transition's direction, if it has one.
    pub fn direction(&self) -> Option<Direction> {
        match self {
            Transition::MoveIn { direction }
            | Transition::MoveOut { direction }
            | Transition::Push { direction }
            | Transition::SlideIn { direction }
            | Transition::SlideOut { direction } => Some(*direction),
            _ => None,
        }
    }

    /// The same transition with another direction (no-op for directionless ones).
    pub fn with_direction(self, d: Direction) -> Transition {
        match self {
            Transition::MoveIn { .. } => Transition::MoveIn { direction: d },
            Transition::MoveOut { .. } => Transition::MoveOut { direction: d },
            Transition::Push { .. } => Transition::Push { direction: d },
            Transition::SlideIn { .. } => Transition::SlideIn { direction: d },
            Transition::SlideOut { .. } => Transition::SlideOut { direction: d },
            other => other,
        }
    }

    /// Whether a drag of `(dx, dy)` (screen space, y down) starts it on an "on drag" trigger: a
    /// directional transition only for a drag mostly its way (a push left for a drag to the left),
    /// the others for a drag any way.
    pub fn accepts_drag(&self, dx: f32, dy: f32) -> bool {
        match self.direction() {
            None => true,
            Some(Direction::Left) => dx < 0.0 && dx.abs() >= dy.abs(),
            Some(Direction::Right) => dx > 0.0 && dx.abs() >= dy.abs(),
            Some(Direction::Top) => dy < 0.0 && dy.abs() >= dx.abs(),
            Some(Direction::Bottom) => dy > 0.0 && dy.abs() >= dx.abs(),
        }
    }

    /// The opposite motion (used when a "while hovering/pressing" state reverts).
    pub fn reversed(self) -> Transition {
        let opp = |d: Direction| match d {
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
            Direction::Top => Direction::Bottom,
            Direction::Bottom => Direction::Top,
        };
        match self {
            Transition::MoveIn { direction } => Transition::MoveOut { direction: opp(direction) },
            Transition::MoveOut { direction } => Transition::MoveIn { direction: opp(direction) },
            Transition::Push { direction } => Transition::Push { direction: opp(direction) },
            Transition::SlideIn { direction } => Transition::SlideOut { direction: opp(direction) },
            Transition::SlideOut { direction } => Transition::SlideIn { direction: opp(direction) },
            other => other,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Transition::Instant => "Instant",
            Transition::Dissolve => "Dissolve",
            Transition::SmartAnimate => "Smart animate",
            Transition::MoveIn { .. } => "Move in",
            Transition::MoveOut { .. } => "Move out",
            Transition::Push { .. } => "Push",
            Transition::SlideIn { .. } => "Slide in",
            Transition::SlideOut { .. } => "Slide out",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum OverlayPosition {
    #[default]
    Center,
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
    Manual,
}

impl OverlayPosition {
    pub const ALL: [OverlayPosition; 8] = [
        OverlayPosition::Center,
        OverlayPosition::TopLeft,
        OverlayPosition::TopCenter,
        OverlayPosition::TopRight,
        OverlayPosition::BottomLeft,
        OverlayPosition::BottomCenter,
        OverlayPosition::BottomRight,
        OverlayPosition::Manual,
    ];

    pub fn label(self) -> &'static str {
        match self {
            OverlayPosition::Center => "Center",
            OverlayPosition::TopLeft => "Top left",
            OverlayPosition::TopCenter => "Top center",
            OverlayPosition::TopRight => "Top right",
            OverlayPosition::BottomLeft => "Bottom left",
            OverlayPosition::BottomCenter => "Bottom center",
            OverlayPosition::BottomRight => "Bottom right",
            OverlayPosition::Manual => "Manual",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    Navigate {
        dest: NodeId,
        /// Keep the scroll position of the frame being left (else the destination starts at the top).
        #[serde(default, alias = "preserveScroll", skip_serializing_if = "is_false")]
        preserve_scroll: bool,
    },
    Back,
    ScrollTo {
        dest: NodeId,
        /// Where the layer ends up from the scrolling frame's top-left (x, y): room left above it,
        /// e.g. for a fixed header.
        #[serde(default, skip_serializing_if = "is_origin")]
        offset: [f64; 2],
    },
    OpenOverlay {
        dest: NodeId,
        #[serde(default)]
        position: OverlayPosition,
        /// A click outside the overlay closes it (on unless turned off, as new overlays start).
        #[serde(default = "crate::node::yes", alias = "closeOnOutside")]
        close_on_outside: bool,
        /// Colour drawn behind the overlay (over the frame underneath); None = no background.
        #[serde(default)]
        background: Option<figcraft_color::Color>,
        /// Manual position: where the overlay's top-left sits from the top-left of the layer that
        /// opened it (x, y). None = right below that layer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<[f64; 2]>,
    },
    SwapOverlay {
        dest: NodeId,
    },
    CloseOverlay,
    OpenUrl {
        url: String,
    },
    /// Change an instance to another variant (interactive components).
    ChangeTo {
        dest: NodeId,
    },
    SetVariable {
        variable: String,
        value: serde_json::Value,
        /// Computed when the action runs, in place of `value` (`count + 1`, `not open`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expr: Option<Expr>,
    },
    /// Switch a variable collection to one of its modes for the rest of the prototype.
    SetVariableMode {
        collection: CollectionId,
        mode: ModeId,
    },
    /// If / else if / else: the actions of the first block whose condition holds run (a block
    /// without a condition is the "else").
    Conditional {
        blocks: Vec<ConditionalBlock>,
    },
}

/// One branch of a conditional action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConditionalBlock {
    /// `None`: always (the "else" block).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Expr>,
    #[serde(default)]
    pub actions: Vec<Action>,
}

impl ConditionalBlock {
    /// The actions of the first block that applies, with variables resolved in `modes`.
    pub fn choose<'a>(blocks: &'a [ConditionalBlock], vars: &Variables, modes: &BTreeMap<CollectionId, ModeId>) -> Option<&'a [Action]> {
        blocks.iter().find(|b| b.condition.as_ref().is_none_or(|c| c.holds(vars, modes))).map(|b| b.actions.as_slice())
    }
}

/// Every action in `actions` and in their conditional blocks (depth-first, in order).
pub fn walk(actions: &[Action]) -> Walk<'_> {
    Walk { stack: vec![(actions.iter(), 0)] }
}

/// Iterator of [`walk`].
pub struct Walk<'a> {
    stack: Vec<(std::slice::Iter<'a, Action>, usize)>,
}

impl<'a> Iterator for Walk<'a> {
    type Item = &'a Action;
    fn next(&mut self) -> Option<&'a Action> {
        loop {
            let (it, depth) = self.stack.last_mut()?;
            let depth = *depth;
            let Some(a) = it.next() else {
                self.stack.pop();
                continue;
            };
            if let Action::Conditional { blocks } = a
                && depth < MAX_ACTION_DEPTH
            {
                self.stack.extend(blocks.iter().rev().map(|b| (b.actions.iter(), depth + 1)));
            }
            return Some(a);
        }
    }
}

/// Call `f` on every action in `actions` and in their conditional blocks.
pub fn walk_mut(actions: &mut [Action], f: &mut dyn FnMut(&mut Action)) {
    walk_mut_at(actions, f, 0);
}

fn walk_mut_at(actions: &mut [Action], f: &mut dyn FnMut(&mut Action), depth: usize) {
    for a in actions {
        f(a);
        if let Action::Conditional { blocks } = a
            && depth < MAX_ACTION_DEPTH
        {
            for b in blocks {
                walk_mut_at(&mut b.actions, f, depth + 1);
            }
        }
    }
}

/// Keep the actions `keep` accepts, inside conditional blocks too; conditionals left with no
/// action in any block go as well.
pub fn retain_deep(actions: &mut Vec<Action>, keep: &mut dyn FnMut(&Action) -> bool) {
    retain_at(actions, keep, 0);
}

fn retain_at(actions: &mut Vec<Action>, keep: &mut dyn FnMut(&Action) -> bool, depth: usize) {
    actions.retain_mut(|a| {
        if !keep(a) {
            return false;
        }
        match a {
            Action::Conditional { blocks } if depth < MAX_ACTION_DEPTH => {
                for b in blocks.iter_mut() {
                    retain_at(&mut b.actions, keep, depth + 1);
                }
                blocks.iter().any(|b| !b.actions.is_empty())
            }
            Action::Conditional { .. } => false,
            _ => true,
        }
    });
}

impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Action::Navigate { .. } => "Navigate to",
            Action::Back => "Back",
            Action::ScrollTo { .. } => "Scroll to",
            Action::OpenOverlay { .. } => "Open overlay",
            Action::SwapOverlay { .. } => "Swap overlay",
            Action::CloseOverlay => "Close overlay",
            Action::OpenUrl { .. } => "Open link",
            Action::ChangeTo { .. } => "Change to",
            Action::SetVariable { .. } => "Set variable",
            Action::SetVariableMode { .. } => "Set variable mode",
            Action::Conditional { .. } => "Conditional",
        }
    }

    /// The variables the action (and what it nests) sets or reads.
    pub fn variables(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for a in walk(std::slice::from_ref(self)) {
            match a {
                Action::SetVariable { variable, expr, .. } => {
                    out.push(variable.as_str());
                    out.extend(expr.iter().flat_map(Expr::variables));
                }
                Action::Conditional { blocks } => out.extend(blocks.iter().filter_map(|b| b.condition.as_ref()).flat_map(Expr::variables)),
                _ => {}
            }
        }
        out
    }
    pub fn dest(&self) -> Option<NodeId> {
        match self {
            Action::Navigate { dest, .. }
            | Action::ScrollTo { dest, .. }
            | Action::OpenOverlay { dest, .. }
            | Action::SwapOverlay { dest }
            | Action::ChangeTo { dest } => Some(*dest),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Interaction {
    #[serde(default)]
    pub trigger: Trigger,
    pub actions: Vec<Action>,
    #[serde(default)]
    pub transition: Transition,
    #[serde(default = "default_duration", alias = "durationMs")]
    pub duration_ms: u32,
    #[serde(default)]
    pub easing: Easing,
    /// Mouse enter, leave, down and up: how long after the gesture the actions run (ms).
    #[serde(default, alias = "delayMs", skip_serializing_if = "is_zero")]
    pub delay_ms: u32,
}

impl Interaction {
    /// Seconds between the gesture and the actions: the delay of a mouse enter, leave, down or up
    /// trigger (other triggers have none; "after delay" times itself).
    pub fn delay_secs(&self) -> f64 {
        match self.trigger {
            Trigger::MouseEnter | Trigger::MouseLeave | Trigger::MouseDown | Trigger::MouseUp => f64::from(self.delay_ms.min(MAX_DELAY_MS)) / 1000.0,
            _ => 0.0,
        }
    }
}

/// Longest delay a trigger waits (ms).
pub const MAX_DELAY_MS: u32 = 600_000;

fn default_duration() -> u32 {
    300
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_origin(p: &[f64; 2]) -> bool {
    *p == [0.0, 0.0]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Overflow {
    #[default]
    None,
    Horizontal,
    Vertical,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ScrollPosition {
    #[default]
    Scrolls,
    Fixed,
    Sticky,
}

/// A prototype flow's starting point on a page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Flow {
    pub name: String,
    pub start: NodeId,
    #[serde(default)]
    pub description: String,
}

/// How the prototype device is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRotation {
    #[default]
    Portrait,
    /// Turned a quarter turn counter-clockwise: the screen is wider than it is tall.
    Landscape,
}

impl DeviceRotation {
    /// From a command or file value: `portrait` / `landscape`, or the angle in degrees (0, 90).
    pub fn parse(v: &str) -> Option<Self> {
        match v.trim().to_ascii_lowercase().as_str() {
            "portrait" | "0" | "none" => Some(Self::Portrait),
            "landscape" | "90" | "ccw_90" => Some(Self::Landscape),
            _ => None,
        }
    }
}

/// Prototype device settings on a page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrototypeSettings {
    pub device: String,
    pub background: figcraft_color::Color,
    /// Applies to devices that turn (phones and tablets).
    pub rotation: DeviceRotation,
    /// The device's colour: one of its model's [`Device::colors`] by name (empty or one the model
    /// doesn't come in: its first).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub color: String,
    /// The screen of the [`CUSTOM_DEVICE`](Self::CUSTOM_DEVICE) (points, width × height).
    #[serde(alias = "customSize", skip_serializing_if = "is_custom_default")]
    pub custom_size: [f64; 2],
    /// Responsive: the presented frame is resized to the screen (the device's, else the window)
    /// and its constraints and auto layout respond, instead of showing at its own size.
    #[serde(skip_serializing_if = "is_false")]
    pub responsive: bool,
}

impl Default for PrototypeSettings {
    fn default() -> Self {
        Self {
            device: "None".into(),
            background: figcraft_color::Color::from_rgb8_const(0x1E, 0x1E, 0x1E),
            rotation: DeviceRotation::Portrait,
            color: String::new(),
            custom_size: PrototypeSettings::CUSTOM_DEFAULT,
            responsive: false,
        }
    }
}

fn is_custom_default(s: &[f64; 2]) -> bool {
    *s == PrototypeSettings::CUSTOM_DEFAULT
}

/// A colour a device model comes in (its body; the glass around the screen stays black).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceColor {
    pub name: &'static str,
    pub rgb: [u8; 3],
}

const fn c(name: &'static str, r: u8, g: u8, b: u8) -> DeviceColor {
    DeviceColor { name, rgb: [r, g, b] }
}

/// A device prototypes are presented on: its name and screen size in points (held upright).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Device {
    pub name: &'static str,
    pub width: f64,
    pub height: f64,
    /// It can be turned to landscape (phones and tablets; laptops and desktops can't).
    pub rotates: bool,
    /// The colours it comes in, the default first (none for a custom size: just a screen).
    pub colors: &'static [DeviceColor],
}

impl PrototypeSettings {
    /// The devices to pick from ("None" presents the frame at its own size).
    pub const DEVICES: [Device; 19] = [
        Device {
            name: "iPhone 17",
            width: 402.0,
            height: 874.0,
            rotates: true,
            colors: &[c("Black", 0x38, 0x39, 0x3D), c("White", 0xE9, 0xE9, 0xE5)],
        },
        Device {
            name: "iPhone 17 Pro",
            width: 402.0,
            height: 874.0,
            rotates: true,
            colors: &[c("Black", 0x38, 0x39, 0x3D), c("Silver", 0xD7, 0xD7, 0xD5)],
        },
        Device {
            name: "iPhone 17 Pro Max",
            width: 440.0,
            height: 956.0,
            rotates: true,
            colors: &[c("Black", 0x38, 0x39, 0x3D), c("Silver", 0xD7, 0xD7, 0xD5)],
        },
        Device {
            name: "iPhone Air",
            width: 420.0,
            height: 912.0,
            rotates: true,
            colors: &[c("Black", 0x38, 0x39, 0x3D), c("Silver", 0xD7, 0xD7, 0xD5)],
        },
        Device {
            name: "iPhone 16",
            width: 393.0,
            height: 852.0,
            rotates: true,
            colors: &[
                c("Black", 0x3A, 0x3B, 0x3F),
                c("White", 0xEE, 0xED, 0xE8),
                c("Pink", 0xEC, 0xBF, 0xCB),
                c("Teal", 0xA3, 0xCF, 0xC6),
                c("Blue", 0x72, 0x84, 0xD4),
            ],
        },
        Device {
            name: "iPhone 16 Pro",
            width: 402.0,
            height: 874.0,
            rotates: true,
            colors: &[c("Black", 0x3B, 0x3A, 0x38), c("White", 0xEC, 0xEB, 0xE6)],
        },
        Device {
            name: "iPhone 16 Pro Max",
            width: 440.0,
            height: 956.0,
            rotates: true,
            colors: &[c("Black", 0x3B, 0x3A, 0x38), c("White", 0xEC, 0xEB, 0xE6), c("Natural", 0xBF, 0xB9, 0xAF), c("Sand", 0xC6, 0xAC, 0x8F)],
        },
        Device {
            name: "iPhone 16 Plus",
            width: 430.0,
            height: 932.0,
            rotates: true,
            colors: &[c("Black", 0x3A, 0x3B, 0x3F), c("White", 0xEE, 0xED, 0xE8)],
        },
        Device {
            name: "iPhone 15 Pro",
            width: 393.0,
            height: 852.0,
            rotates: true,
            colors: &[c("Black", 0x3B, 0x3A, 0x38), c("Silver", 0xD7, 0xD7, 0xD5)],
        },
        Device {
            name: "iPhone 15 Pro Max",
            width: 430.0,
            height: 932.0,
            rotates: true,
            colors: &[c("Black", 0x3B, 0x3A, 0x38), c("Silver", 0xD7, 0xD7, 0xD5)],
        },
        Device {
            name: "iPhone 15 Plus",
            width: 430.0,
            height: 932.0,
            rotates: true,
            colors: &[c("Black", 0x3A, 0x3B, 0x3F), c("White", 0xEE, 0xED, 0xE8)],
        },
        Device {
            name: "iPhone 14 Pro",
            width: 393.0,
            height: 852.0,
            rotates: true,
            colors: &[c("Black", 0x3B, 0x3A, 0x38), c("Silver", 0xD7, 0xD7, 0xD5)],
        },
        Device {
            name: "iPhone 14 Pro Max",
            width: 430.0,
            height: 932.0,
            rotates: true,
            colors: &[c("Black", 0x3B, 0x3A, 0x38), c("Silver", 0xD7, 0xD7, 0xD5)],
        },
        Device {
            name: "iPhone 14",
            width: 390.0,
            height: 844.0,
            rotates: true,
            colors: &[c("Black", 0x3A, 0x3B, 0x3F), c("White", 0xEE, 0xED, 0xE8)],
        },
        Device {
            name: "Android Compact",
            width: 412.0,
            height: 917.0,
            rotates: true,
            colors: &[c("Black", 0x2E, 0x2F, 0x33), c("Silver", 0xD6, 0xD8, 0xDB), c("Green", 0xA6, 0xC0, 0xA3)],
        },
        Device {
            name: "Android Medium",
            width: 700.0,
            height: 840.0,
            rotates: true,
            colors: &[c("Black", 0x2E, 0x2F, 0x33), c("Silver", 0xD6, 0xD8, 0xDB)],
        },
        Device {
            name: "iPad mini",
            width: 744.0,
            height: 1133.0,
            rotates: true,
            colors: &[c("Grey", 0x6A, 0x6B, 0x6F), c("Silver", 0xE0, 0xE1, 0xE3), c("Blue", 0x9C, 0xB1, 0xC9), c("Purple", 0xC6, 0xBC, 0xD7)],
        },
        Device {
            name: "MacBook Air",
            width: 1280.0,
            height: 832.0,
            rotates: false,
            colors: &[c("Silver", 0xC9, 0xCA, 0xCE), c("Grey", 0x7B, 0x7C, 0x80), c("Dark blue", 0x2E, 0x33, 0x3D), c("Cream", 0xE0, 0xD9, 0xCB)],
        },
        Device {
            name: "Desktop",
            width: 1440.0,
            height: 1024.0,
            rotates: false,
            colors: &[c("Silver", 0xB8, 0xB9, 0xBD), c("Black", 0x2C, 0x2C, 0x2E)],
        },
    ];

    /// The device whose screen is [`custom_size`](Self::custom_size) (no body is drawn around it).
    pub const CUSTOM_DEVICE: &str = "Custom size";
    /// A new custom size.
    pub const CUSTOM_DEFAULT: [f64; 2] = [1280.0, 720.0];
    /// The largest side a custom size may have (points).
    pub const CUSTOM_MAX: f64 = 10_000.0;

    /// A custom screen side from a command or file value: finite, at least 1, at most
    /// [`CUSTOM_MAX`](Self::CUSTOM_MAX).
    pub fn custom_side(v: f64) -> Option<f64> {
        (v.is_finite() && v >= 1.0).then(|| v.min(Self::CUSTOM_MAX).round())
    }

    /// The chosen device model, held upright (None for "None" or a name we don't know).
    pub fn model(&self) -> Option<Device> {
        if self.device == Self::CUSTOM_DEVICE {
            let ([w, h], [dw, dh]) = (self.custom_size, Self::CUSTOM_DEFAULT);
            let (width, height) = (Self::custom_side(w).unwrap_or(dw), Self::custom_side(h).unwrap_or(dh));
            return Some(Device { name: Self::CUSTOM_DEVICE, width, height, rotates: false, colors: &[] });
        }
        Self::DEVICES.iter().find(|d| d.name == self.device).copied()
    }

    /// The chosen device's colour (its model's first unless another is picked; None without a
    /// body).
    pub fn device_color(&self) -> Option<DeviceColor> {
        let colors = self.model()?.colors;
        colors.iter().find(|c| c.name == self.color).or(colors.first()).copied()
    }

    /// The device turned to landscape (only devices that turn do).
    pub fn landscape(&self) -> bool {
        self.rotation == DeviceRotation::Landscape && self.model().is_some_and(|d| d.rotates)
    }

    /// The chosen device as presented: its screen is wider than tall when turned to landscape.
    pub fn device(&self) -> Option<Device> {
        let d = self.model()?;
        Some(if self.landscape() { Device { width: d.height, height: d.width, ..d } } else { d })
    }
}
