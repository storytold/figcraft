//! Prototyping: layers' `interactions` (triggers, actions, transitions and easing) and the legacy
//! `transitionNodeID`, scrolling (`overflowDirection`, `scrollBehavior`), and the pages'
//! `prototypeDevice`, `prototypeBackgrounds` and `prototypeStartNodeID`.

use figcraft_doc::proto::{
    Action, ConditionalBlock, DeviceRotation, Direction, Easing, Expr, ExprOp, Interaction, KeyMods, MAX_ACTION_DEPTH, MAX_EXPR_DEPTH, Overflow,
    OverlayAnchor, OverlayPosition, PrototypeSettings, ScrollPosition, Transition, Trigger, gamepad_button,
};
use figcraft_doc::{Node, NodeId, VariableType, VariableValue};
use serde_json::{Value, json};

use super::Ctx;
use super::props::{color, flag, list, num, text};
use super::variables;

/// Farthest a manual overlay sits from the layer that opens it (page units, each way).
const MAX_OFFSET: f64 = 100_000.0;
/// Longest transition or delay kept (ms).
const MAX_MS: f64 = 60_000.0;
/// Most blocks of one conditional and arguments of one expression function read.
const MAX_BLOCKS: usize = 64;
const MAX_ARGS: usize = 64;
/// Longest URL kept (characters).
const MAX_URL: usize = 4096;
/// Default transition length when the file names none (ms).
const DEFAULT_MS: u32 = 300;

/// Scrolling of a frame and how a layer behaves while its parent scrolls.
pub(super) fn scrolling(n: &mut Node, j: &Value) {
    n.overflow = match text(j, "overflowDirection") {
        Some("HORIZONTAL_SCROLLING") => Overflow::Horizontal,
        Some("VERTICAL_SCROLLING") => Overflow::Vertical,
        Some("HORIZONTAL_AND_VERTICAL_SCROLLING") => Overflow::Both,
        _ => Overflow::None,
    };
    n.scroll_position = match text(j, "scrollBehavior") {
        Some("FIXED") => ScrollPosition::Fixed,
        Some("STICKY_SCROLLS") => ScrollPosition::Sticky,
        Some(_) => ScrollPosition::Scrolls,
        // The deprecated flag, for files written before `scrollBehavior`.
        None if flag(j, "isFixed") == Some(true) => ScrollPosition::Fixed,
        None => ScrollPosition::Scrolls,
    };
}

/// A page's prototype device (and its rotation) and background, when the file sets them.
pub(super) fn page_settings(c: &Value) -> Option<PrototypeSettings> {
    let device = c.get("prototypeDevice").filter(|d| d.is_object());
    let background = list(c, "prototypeBackgrounds").first().and_then(color).filter(|b| b.a > 0.0);
    if device.is_none() && background.is_none() {
        return None;
    }
    let mut s = PrototypeSettings::default();
    if let Some(d) = device {
        s.device = device_name(d).to_string();
        s.rotation = text(d, "rotation").and_then(DeviceRotation::parse).unwrap_or_default();
        // A custom size keeps its screen (turned: its sides swap, as it doesn't turn here).
        if let Some((w, h)) = custom_size(d) {
            s.device = PrototypeSettings::CUSTOM_DEVICE.into();
            s.custom_size = if s.rotation == DeviceRotation::Landscape { [h, w] } else { [w, h] };
            s.rotation = DeviceRotation::Portrait;
        }
    }
    if let Some(b) = background {
        s.background = b.with_alpha(1.0);
    }
    Some(s)
}

/// The screen of a `CUSTOM` device, when its size is usable.
fn custom_size(d: &Value) -> Option<(f64, f64)> {
    if text(d, "type") != Some("CUSTOM") {
        return None;
    }
    let s = d.get("size")?;
    Some((PrototypeSettings::custom_side(num(s, "width")?)?, PrototypeSettings::custom_side(num(s, "height")?)?))
}

/// The closest of our device frames: preset identifiers by the device family they name, unknown
/// presets (and custom sizes without a usable size) by their shorter side.
fn device_name(d: &Value) -> &'static str {
    let size = d.get("size").and_then(|s| Some((num(s, "width")?, num(s, "height")?)));
    let by_size = || match size {
        Some((w, h)) if w > 0.0 && h > 0.0 => {
            let (short, long) = (w.min(h), w.max(h));
            if short <= 500.0 {
                if long >= 900.0 { "iPhone 16 Pro Max" } else { "iPhone 16" }
            } else if long <= 1400.0 && long / short < 1.55 {
                // Tablets are squarer than laptop and desktop screens.
                "iPad mini"
            } else {
                "Desktop"
            }
        }
        _ => "None",
    };
    match text(d, "type") {
        Some("PRESET") => {
            let id = text(d, "presetIdentifier").unwrap_or("").to_ascii_uppercase();
            let has = |words: &[&str]| words.iter().any(|w| id.contains(w));
            if has(&["IPAD", "TABLET", "GALAXY_TAB", "SURFACE_PRO"]) {
                "iPad mini"
            } else if has(&["MACBOOK", "LAPTOP"]) {
                "MacBook Air"
            } else if has(&["DESKTOP", "IMAC", "MONITOR", "BROWSER", "TV"]) {
                "Desktop"
            } else if has(&["ANDROID", "PIXEL", "GALAXY"]) {
                "Android Compact"
            } else if has(&["IPHONE"]) {
                if has(&["MAX", "PLUS"]) { "iPhone 16 Pro Max" } else { "iPhone 16" }
            } else {
                by_size()
            }
        }
        Some("CUSTOM") => by_size(),
        _ => "None",
    }
}

impl Ctx<'_> {
    /// The layer's interactions (`interactions`, else the legacy `transitionNodeID`), with
    /// destinations resolved to imported layers.
    pub(super) fn interactions(&mut self, j: &Value) -> Vec<Interaction> {
        let list_ = list(j, "interactions");
        if list_.is_empty() {
            return self.legacy_transition(j).into_iter().collect();
        }
        list_.iter().filter_map(|it| self.interaction(it)).collect()
    }

    fn legacy_transition(&mut self, j: &Value) -> Option<Interaction> {
        let dest = self.dest(text(j, "transitionNodeID")?)?;
        let ms = num(j, "transitionDuration").map(|d| d.clamp(0.0, MAX_MS) as u32).unwrap_or(0);
        Some(Interaction {
            trigger: Trigger::OnClick,
            actions: vec![Action::Navigate { dest, preserve_scroll: false }],
            transition: if ms > 0 { Transition::Dissolve } else { Transition::Instant },
            duration_ms: if ms > 0 { ms } else { DEFAULT_MS },
            easing: easing_type(text(j, "transitionEasing"), None),
            delay_ms: 0,
        })
    }

    fn interaction(&mut self, it: &Value) -> Option<Interaction> {
        let t = it.get("trigger").filter(|t| t.is_object())?;
        let trigger = self.trigger(t)?;
        // Mouse enter, leave, down and up wait before they act (`.fig` files name it `timeout`).
        let delay_ms = num(t, "delay").or_else(|| num(t, "timeout")).filter(|d| d.is_finite()).map_or(0, |d| d.clamp(0.0, MAX_MS) as u32);
        let mut actions = Vec::new();
        let mut transition: Option<&Value> = None;
        for a in list(it, "actions") {
            self.action(a, &mut actions, &mut transition, 0);
        }
        if actions.is_empty() {
            self.warn("prototype interactions without supported actions were skipped");
            return None;
        }
        let (transition, duration_ms, easing) = match transition {
            Some(t) => self.transition(t),
            None => (Transition::Instant, DEFAULT_MS, Easing::default()),
        };
        let mut it = Interaction { trigger, actions, transition, duration_ms, easing, delay_ms };
        if it.delay_secs() == 0.0 {
            it.delay_ms = 0;
        }
        Some(it)
    }

    fn trigger(&mut self, t: &Value) -> Option<Trigger> {
        match text(t, "type") {
            Some("ON_CLICK") => Some(Trigger::OnClick),
            Some("ON_HOVER") => Some(Trigger::OnHover),
            Some("ON_PRESS") => Some(Trigger::OnPress),
            Some("ON_DRAG") => Some(Trigger::OnDrag),
            Some("AFTER_TIMEOUT") => Some(Trigger::AfterDelay { ms: num(t, "timeout").unwrap_or(0.0).clamp(0.0, MAX_MS) as u32 }),
            Some("MOUSE_ENTER") => Some(Trigger::MouseEnter),
            Some("MOUSE_LEAVE") => Some(Trigger::MouseLeave),
            Some("MOUSE_DOWN") => Some(Trigger::MouseDown),
            Some("MOUSE_UP") => Some(Trigger::MouseUp),
            Some(ty @ ("ON_KEY_DOWN" | "ON_KEY_UP")) => {
                let codes: Vec<f64> = list(t, "keyCodes").iter().filter_map(Value::as_f64).collect();
                // Controllers: their codes read as standard-layout button indices; one button each.
                if text(t, "device").is_some_and(|d| d != "KEYBOARD") {
                    let button = match codes.as_slice() {
                        [c] if c.fract() == 0.0 && *c >= 0.0 => gamepad_button(*c as usize),
                        _ => None,
                    };
                    if button.is_none() {
                        self.warn("gamepad triggers without exactly one known controller button were skipped");
                    } else if ty == "ON_KEY_UP" {
                        self.warn("key up triggers import as key presses");
                    }
                    return button.map(|b| Trigger::KeyPress { key: b.to_string() });
                }
                let Some(key) = codes.iter().rev().find(|c| modifier(**c).is_none()).and_then(|c| key_name(*c)) else {
                    self.warn("key triggers without a supported key were skipped");
                    return None;
                };
                // Modifier keys make it a combination (`Shift+A`).
                let mut mods = KeyMods::default();
                for m in codes.iter().filter_map(|c| modifier(*c)) {
                    *match m {
                        16 => &mut mods.shift,
                        17 => &mut mods.ctrl,
                        18 => &mut mods.alt,
                        _ => &mut mods.cmd,
                    } = true;
                }
                let key = mods.combo(&key);
                if ty == "ON_KEY_UP" {
                    self.warn("key up triggers import as key presses");
                }
                Some(Trigger::KeyPress { key })
            }
            Some("ON_MEDIA_HIT" | "ON_MEDIA_END") => {
                self.warn("video triggers are not supported and were skipped");
                None
            }
            _ => {
                self.warn("prototype triggers of unknown types were skipped");
                None
            }
        }
    }

    /// Append the action `a` maps to (a lone conditional block without a condition is inlined).
    fn action<'v>(&mut self, a: &'v Value, out: &mut Vec<Action>, transition: &mut Option<&'v Value>, depth: usize) {
        let mapped = match text(a, "type") {
            Some("BACK") => Some(Action::Back),
            Some("CLOSE") => Some(Action::CloseOverlay),
            Some("URL") => text(a, "url").filter(|u| !u.trim().is_empty()).map(|u| Action::OpenUrl {
                url: u.chars().take(MAX_URL).collect(),
                // A new tab unless the action says otherwise.
                new_tab: a.get("openInNewTab").and_then(Value::as_bool).unwrap_or(true),
            }),
            Some("NODE") => {
                let mapped = self.node_action(a);
                if mapped.is_some() && transition.is_none() {
                    *transition = a.get("transition").filter(|t| t.is_object());
                }
                mapped
            }
            Some("SET_VARIABLE") => self.set_variable(a),
            Some("SET_VARIABLE_MODE") => self.set_variable_mode(a),
            Some("CONDITIONAL") if depth >= MAX_ACTION_DEPTH => {
                self.warn("conditional prototype actions nested too deeply were skipped");
                None
            }
            Some("CONDITIONAL") => match list(a, "conditionalBlocks") {
                // A lone block without a condition always runs.
                [only] if only.get("condition").is_none_or(Value::is_null) => {
                    for inner in list(only, "actions") {
                        self.action(inner, out, transition, depth + 1);
                    }
                    None
                }
                blocks => self.conditional(blocks, transition, depth),
            },
            Some("UPDATE_MEDIA_RUNTIME") => {
                self.warn("video actions are not supported and were skipped");
                None
            }
            _ => {
                self.warn("prototype actions of unknown types were skipped");
                None
            }
        };
        out.extend(mapped);
    }

    /// If / else if / else blocks; skipped whole when a condition can't be imported (running the
    /// wrong branch would be worse than none).
    fn conditional<'v>(&mut self, blocks: &'v [Value], transition: &mut Option<&'v Value>, depth: usize) -> Option<Action> {
        let mut out = Vec::new();
        for b in blocks.iter().take(MAX_BLOCKS) {
            let condition = match b.get("condition").filter(|c| !c.is_null()) {
                None => None,
                Some(c) => match self.expr(c, 0) {
                    Some(e) => Some(e),
                    None => {
                        self.warn("conditional prototype actions with conditions that can't be evaluated were skipped");
                        return None;
                    }
                },
            };
            let mut actions = Vec::new();
            for inner in list(b, "actions") {
                self.action(inner, &mut actions, transition, depth + 1);
            }
            let otherwise = condition.is_none();
            out.push(ConditionalBlock { condition, actions });
            if otherwise {
                // Blocks after the "else" never run.
                break;
            }
        }
        out.iter().any(|b| !b.actions.is_empty()).then_some(Action::Conditional { blocks: out })
    }

    fn node_action(&mut self, a: &Value) -> Option<Action> {
        let Some(dest) = text(a, "destinationId").and_then(|d| self.dest(d)) else {
            self.warn("prototype actions without a destination in the file were skipped");
            return None;
        };
        Some(match text(a, "navigation") {
            Some("NAVIGATE") | None => {
                Action::Navigate { dest, preserve_scroll: a.get("preserveScrollPosition").and_then(Value::as_bool).unwrap_or(false) }
            }
            Some("SWAP") => Action::SwapOverlay { dest },
            Some("SCROLL_TO") => Action::ScrollTo { dest, offset: [0.0, 0.0] },
            Some("CHANGE_TO") => Action::ChangeTo { dest },
            Some("OVERLAY") => {
                let meta = text(a, "destinationId").and_then(|d| self.overlay_meta.get(d)).copied();
                let position = meta.and_then(|m| text(m, "overlayPositionType")).and_then(overlay_position).unwrap_or_default();
                // Overlays close on a click outside unless the destination says otherwise.
                let close_on_outside = meta.and_then(|m| text(m, "overlayBackgroundInteraction")).is_none_or(|i| i == "CLOSE_ON_CLICK_OUTSIDE");
                let background = meta
                    .and_then(|m| m.get("overlayBackground"))
                    .filter(|b| text(b, "type") == Some("SOLID_COLOR"))
                    .and_then(|b| b.get("color"))
                    .and_then(color);
                // A manual overlay sits that far from the top-left of the layer that opens it.
                let offset = a
                    .get("overlayRelativePosition")
                    .filter(|_| position == OverlayPosition::Manual)
                    .and_then(|v| Some([num(v, "x")?, num(v, "y")?]))
                    .filter(|o| o.iter().all(|c| c.is_finite()))
                    .map(|o| o.map(|c| c.clamp(-MAX_OFFSET, MAX_OFFSET)));
                Action::OpenOverlay { dest, position, close_on_outside, background, offset, anchor: OverlayAnchor::Screen }
            }
            Some(_) => {
                self.warn("prototype navigations of unknown types were skipped");
                return None;
            }
        })
    }

    fn set_variable(&mut self, a: &Value) -> Option<Action> {
        let Some((id, kind)) = text(a, "variableId").and_then(|v| self.var_kinds.get_key_value(v)) else {
            self.warn("set-variable actions on variables that aren't in the variables JSON were skipped");
            return None;
        };
        let (variable, kind) = (id.clone(), *kind);
        let Some(data) = a.get("variableValue").filter(|d| d.is_object()) else {
            self.warn("set-variable actions without a value were skipped");
            return None;
        };
        let dynamic = data.get("value").is_some_and(|v| variables::alias(v).is_some()) || expr_parts(data).is_some();
        if dynamic {
            // A variable reference or an expression, computed when the action runs.
            let Some(expr) = self.expr(data, 0) else {
                self.warn("set-variable actions with values that can't be evaluated were skipped");
                return None;
            };
            return Some(Action::SetVariable { variable, value: Value::Null, expr: Some(expr) });
        }
        let value = match data.get("value").and_then(|v| variables::value(kind, v)) {
            Some(VariableValue::Color(c)) => json!(c.to_css_hex()),
            Some(VariableValue::Number(n)) => json!(n),
            Some(VariableValue::String(s)) => json!(s),
            Some(VariableValue::Boolean(b)) => json!(b),
            Some(VariableValue::Alias(_) | VariableValue::ColorAlpha { .. } | VariableValue::ColorMix { .. }) | None => {
                self.warn("set-variable actions with values that don't fit the variable were skipped");
                return None;
            }
        };
        Some(Action::SetVariable { variable, value, expr: None })
    }

    fn set_variable_mode(&mut self, a: &Value) -> Option<Action> {
        let collection = text(a, "variableCollectionId");
        let mode = text(a, "variableModeId");
        match (collection, mode) {
            (Some(c), Some(m)) if self.var_modes.get(c).is_some_and(|modes| modes.iter().any(|x| x == m)) => {
                Some(Action::SetVariableMode { collection: c.to_string(), mode: m.to_string() })
            }
            _ => {
                self.warn("set-variable-mode actions on collections or modes that aren't in the variables JSON were skipped");
                None
            }
        }
    }

    /// A `VariableData` value (literal, variable reference or expression) as an expression.
    fn expr(&mut self, data: &Value, depth: usize) -> Option<Expr> {
        if depth > MAX_EXPR_DEPTH {
            return None;
        }
        if let Some((function, args)) = expr_parts(data) {
            if function == "VAR_MODE_LOOKUP" {
                return self.mode_lookup(args);
            }
            let op = match function {
                "ADDITION" => ExprOp::Add,
                "SUBTRACTION" => ExprOp::Subtract,
                "MULTIPLICATION" => ExprOp::Multiply,
                "DIVISION" => ExprOp::Divide,
                "EQUALS" => ExprOp::Equals,
                "NOT_EQUAL" => ExprOp::NotEqual,
                "LESS_THAN" => ExprOp::Less,
                "LESS_THAN_OR_EQUAL" => ExprOp::LessOrEqual,
                "GREATER_THAN" => ExprOp::Greater,
                "GREATER_THAN_OR_EQUAL" => ExprOp::GreaterOrEqual,
                "AND" => ExprOp::And,
                "OR" => ExprOp::Or,
                "NOT" => ExprOp::Not,
                "NEGATE" => ExprOp::Negate,
                _ => {
                    self.warn("prototype expressions with unsupported functions were skipped");
                    return None;
                }
            };
            if args.is_empty() || args.len() > MAX_ARGS {
                return None;
            }
            let args = args.iter().map(|x| self.expr(x, depth + 1)).collect::<Option<Vec<Expr>>>()?;
            return Some(Expr::op(op, args));
        }
        let v = data.get("value")?;
        if let Some(id) = variables::alias(v) {
            if !self.var_kinds.contains_key(id) {
                self.warn("prototype expressions on variables that aren't in the variables JSON were skipped");
                return None;
            }
            return Some(Expr::var(id));
        }
        let kind = match text(data, "resolvedType").or_else(|| text(data, "type")).and_then(variables::kind_of) {
            Some(k) => k,
            // Untyped: from the JSON value itself.
            None => match v {
                Value::Bool(_) => VariableType::Boolean,
                Value::Number(_) => VariableType::Number,
                Value::String(_) => VariableType::String,
                _ => VariableType::Color,
            },
        };
        variables::value(kind, v).map(Expr::Value)
    }

    /// A variable mode lookup (`VAR_MODE_LOOKUP` over a variable reference and the id of one of its
    /// collection's modes): the variable's value in that mode.
    fn mode_lookup(&mut self, args: &[Value]) -> Option<Expr> {
        let parts = match args {
            [var, mode] => var.get("value").and_then(variables::alias).zip(mode.get("value").and_then(Value::as_str)),
            _ => None,
        };
        let vars = &self.doc.variables;
        let found = parts.filter(|(id, _)| self.var_kinds.contains_key(*id)).and_then(|(id, mode)| {
            let v = vars.get_by_id(id)?;
            let m = vars.collection_by_id(&v.collection)?.mode_by_id(mode)?;
            Some(Expr::InMode { variable: v.id.clone(), mode: m.id.clone() })
        });
        if found.is_none() {
            self.warn("variable mode lookups in prototype expressions that aren't a variable and one of its modes were skipped");
        }
        found
    }

    /// The imported layer a REST id names.
    fn dest(&self, rest_id: &str) -> Option<NodeId> {
        self.ids.get(rest_id).copied()
    }

    /// Transition, duration (ms) and easing of a `transition` object.
    fn transition(&mut self, t: &Value) -> (Transition, u32, Easing) {
        let direction = match text(t, "direction") {
            Some("RIGHT") => Direction::Right,
            Some("TOP") => Direction::Top,
            Some("BOTTOM") => Direction::Bottom,
            _ => Direction::Left,
        };
        let tr = match text(t, "type") {
            Some("DISSOLVE") => Transition::Dissolve,
            // Animated scrolling: any animated transition scrolls smoothly.
            Some("SMART_ANIMATE" | "SCROLL_ANIMATE") => Transition::SmartAnimate,
            Some("MOVE_IN") => Transition::MoveIn { direction },
            Some("MOVE_OUT") => Transition::MoveOut { direction },
            Some("PUSH") => Transition::Push { direction },
            Some("SLIDE_IN") => Transition::SlideIn { direction },
            Some("SLIDE_OUT") => Transition::SlideOut { direction },
            _ => {
                self.warn("prototype transitions of unknown types play instantly");
                Transition::Instant
            }
        };
        let ms = num(t, "duration").map_or(DEFAULT_MS, |d| d.clamp(0.0, MAX_MS) as u32);
        let e = t.get("easing");
        (tr, ms, easing_type(e.and_then(|e| text(e, "type")), e))
    }
}

/// The function name and arguments of an `EXPRESSION` value.
fn expr_parts(data: &Value) -> Option<(&str, &[Value])> {
    let e = data.get("value").filter(|v| v.get("expressionFunction").is_some())?;
    Some((text(e, "expressionFunction")?, list(e, "expressionArguments")))
}

/// An easing curve by its type name, with custom parameters from `e`.
fn easing_type(ty: Option<&str>, e: Option<&Value>) -> Easing {
    let field = |obj: &str, k: &str| e.and_then(|e| e.get(obj)).and_then(|o| num(o, k));
    match ty {
        Some("LINEAR") => Easing::Linear,
        Some("EASE_IN") => Easing::EaseIn,
        Some("EASE_IN_AND_OUT") => Easing::EaseInOut,
        Some("EASE_IN_BACK") => Easing::EaseInBack,
        Some("EASE_OUT_BACK") => Easing::EaseOutBack,
        Some("EASE_IN_AND_OUT_BACK") => Easing::EaseInOutBack,
        Some("GENTLE" | "GENTLE_SPRING") => Easing::Gentle,
        Some("QUICK") => Easing::Quick,
        Some("BOUNCY") => Easing::Bouncy,
        Some("SLOW") => Easing::Slow,
        Some("CUSTOM_CUBIC_BEZIER") => {
            let b = |k: &str| field("easingFunctionCubicBezier", k);
            match (b("x1"), b("y1"), b("x2"), b("y2")) {
                (Some(x1), Some(y1), Some(x2), Some(y2)) => {
                    Easing::CustomBezier { x1: x1.clamp(0.0, 1.0), y1: y1.clamp(-10.0, 10.0), x2: x2.clamp(0.0, 1.0), y2: y2.clamp(-10.0, 10.0) }
                }
                _ => Easing::CUSTOM_BEZIER,
            }
        }
        Some("CUSTOM_SPRING") => {
            let s = |k: &str| field("easingFunctionSpring", k);
            match (s("mass"), s("stiffness"), s("damping")) {
                (Some(m), Some(k), Some(d)) if m > 0.0 && k > 0.0 && d >= 0.0 => {
                    Easing::CustomSpring { mass: m.min(1e4), stiffness: k.min(1e6), damping: d.min(1e6) }
                }
                _ => Easing::CUSTOM_SPRING,
            }
        }
        _ => Easing::EaseOut,
    }
}

pub(crate) fn overlay_position(s: &str) -> Option<OverlayPosition> {
    Some(match s {
        "CENTER" => OverlayPosition::Center,
        "TOP_LEFT" => OverlayPosition::TopLeft,
        "TOP_CENTER" => OverlayPosition::TopCenter,
        "TOP_RIGHT" => OverlayPosition::TopRight,
        "BOTTOM_LEFT" => OverlayPosition::BottomLeft,
        "BOTTOM_CENTER" => OverlayPosition::BottomCenter,
        "BOTTOM_RIGHT" => OverlayPosition::BottomRight,
        "MANUAL" => OverlayPosition::Manual,
        _ => return None,
    })
}

/// Shift, Control, Alt and the platform keys.
/// A modifier key's code (Shift 16, Control 17, Alt 18, Meta 91/92/93/224).
fn modifier(code: f64) -> Option<i64> {
    Some(code as i64).filter(|c| matches!(c, 16 | 17 | 18 | 91 | 92 | 93 | 224))
}

/// Our key name for a keyboard key code.
pub(crate) fn key_name(code: f64) -> Option<String> {
    if !(code.is_finite() && (0.0..=255.0).contains(&code)) {
        return None;
    }
    let c = code as u32;
    Some(match c {
        8 => "Backspace".into(),
        9 => "Tab".into(),
        13 => "Enter".into(),
        27 => "Escape".into(),
        32 => "Space".into(),
        37 => "ArrowLeft".into(),
        38 => "ArrowUp".into(),
        39 => "ArrowRight".into(),
        40 => "ArrowDown".into(),
        46 => "Delete".into(),
        48..=57 | 65..=90 => char::from_u32(c)?.to_string(),
        112..=123 => format!("F{}", c - 111),
        _ => return None,
    })
}
