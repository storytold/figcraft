use figcraft_doc::proto::{Expr, ExprOp, MAX_ACTION_DEPTH};
use figcraft_doc::{VariableType, VariableValue};

use super::common::key_codes;
use super::{
    AUX, Action, Direction, Easing, Exporter, Fields, Interaction, Kv, LAYERS, Node, NodeId, Overflow, OverlayPosition, ScrollPosition, Transition,
    Trigger, arr, b, color, e, fl, guid, int, o, position, s,
};

impl<'d> Exporter<'d> {
    /// Interactions, scrolling, flow starts and the overlay settings this layer is opened with.
    pub(super) fn prototype(&mut self, n: &Node, f: &mut Fields) {
        self.trigger_size = [n.width, n.height];
        let list: Vec<Kv> = n.interactions.iter().filter_map(|it| self.interaction(it)).collect();
        if !list.is_empty() {
            f.push(("prototypeInteractions", arr(list)));
        }
        let dir = match n.overflow {
            Overflow::None => None,
            Overflow::Horizontal => Some("HORIZONTAL"),
            Overflow::Vertical => Some("VERTICAL"),
            Overflow::Both => Some("BOTH"),
        };
        f.extend(dir.map(|d| ("scrollDirection", e(d))));
        let behavior = match n.scroll_position {
            ScrollPosition::Scrolls => None,
            ScrollPosition::Fixed => Some("FIXED_WHEN_CHILD_OF_SCROLLING_FRAME"),
            ScrollPosition::Sticky => Some("STICKY_SCROLLS"),
        };
        f.extend(behavior.map(|x| ("scrollBehavior", e(x))));
        if let Some((i, name)) = self.starts.get(&n.id) {
            f.push(("prototypeStartingPoint", o(vec![("name", s(name)), ("position", s(&position(*i)))])));
        }
        if let Some(ov) = self.overlays.get(&n.id) {
            const POSITIONS: [&str; 8] = ["CENTER", "TOP_LEFT", "TOP_CENTER", "TOP_RIGHT", "BOTTOM_LEFT", "BOTTOM_CENTER", "BOTTOM_RIGHT", "MANUAL"];
            let pos = POSITIONS.into_iter().find(|p| crate::rest::proto::overlay_position(p) == Some(ov.position)).unwrap_or("CENTER");
            let bg = match ov.background {
                Some(c) => o(vec![("backgroundType", e("SOLID_COLOR")), ("backgroundColor", color(c))]),
                None => o(vec![("backgroundType", e("NONE"))]),
            };
            f.extend([
                ("overlayPositionType", e(pos)),
                ("overlayBackgroundInteraction", e(if ov.close_on_outside { "CLOSE_ON_CLICK_OUTSIDE" } else { "NONE" })),
                ("overlayBackgroundAppearance", bg),
            ]);
        }
    }

    /// Encode the trigger and the actions that can be represented in the export.
    fn interaction(&mut self, it: &Interaction) -> Option<Kv<'static>> {
        let mut event: Fields = Vec::new();
        let ty = match &it.trigger {
            Trigger::OnClick => "ON_CLICK",
            Trigger::OnHover => "ON_HOVER",
            Trigger::OnPress => "ON_PRESS",
            Trigger::OnDrag => "DRAG",
            Trigger::MouseEnter => "MOUSE_IN",
            Trigger::MouseLeave => "MOUSE_OUT",
            Trigger::MouseDown => "MOUSE_DOWN",
            Trigger::MouseUp => "MOUSE_UP",
            Trigger::AfterDelay { ms } => {
                event.push(("transitionTimeout", fl(f64::from(*ms) / 1000.0)));
                "AFTER_TIMEOUT"
            }
            Trigger::KeyPress { key } => {
                let Some((codes, pad)) = key_codes(key) else {
                    self.warn("key triggers on keys the format has no code for were left out");
                    return None;
                };
                let device = e(if pad { "UNKNOWN_CONTROLLER" } else { "KEYBOARD" });
                event.push(("keyTrigger", o(vec![("keyCodes", arr(codes.into_iter().map(int).collect())), ("triggerDevice", device)])));
                "ON_KEY_DOWN"
            }
        };
        // A mouse trigger's delay is the event's timeout.
        if it.delay_secs() > 0.0 {
            event.push(("transitionTimeout", fl(it.delay_secs())));
        }
        event.insert(0, ("interactionType", e(ty)));
        let actions: Vec<Kv> = it.actions.iter().filter_map(|a| self.action(a, it, 0)).collect();
        (!actions.is_empty()).then(|| o(vec![("event", o(event)), ("actions", arr(actions))]))
    }

    /// Encode one action and warn when a variable or conditional action must be omitted.
    fn action(&mut self, a: &Action, it: &Interaction, depth: usize) -> Option<Kv<'static>> {
        let encoded = self.encode_action(a, it, depth);
        if encoded.is_none() && matches!(a, Action::SetVariable { .. } | Action::SetVariableMode { .. } | Action::Conditional { .. }) {
            self.warn("variable or conditional prototype actions with unsupported values, references or nesting were left out");
        }
        encoded
    }

    /// Map an action to Kiwi, preserving branch order and bounding conditional recursion.
    fn encode_action(&mut self, a: &Action, it: &Interaction, depth: usize) -> Option<Kv<'static>> {
        let node = |x: &mut Self, dest: &NodeId, nav: &'static str| -> Option<Kv<'static>> {
            let Some(g) = x.guid_of(*dest) else {
                x.warn("prototype actions whose destination isn't written were left out");
                return None;
            };
            let mut f: Fields = vec![("transitionNodeID", g), ("connectionType", e("INTERNAL_NODE")), ("navigationType", e(nav))];
            x.transition(it, &mut f);
            Some(o(f))
        };
        let plain = |ty: &'static str| Some(o(vec![("connectionType", e(ty))]));
        // Message → layer → interaction → action uses three Kiwi levels. Each
        // conditional adds two, an expression operator adds three, and the deepest
        // condition (an alias with a GUID) uses five levels below its action.
        let expression_levels = (super::super::kiwi::MAX_DEPTH - 8 - 2 * depth) / 3;
        match a {
            Action::Navigate { dest, preserve_scroll } => {
                let mut act = node(self, dest, "NAVIGATE")?;
                if let Kv::Object(f) = &mut act {
                    f.push(("transitionPreserveScroll", b(*preserve_scroll)));
                }
                Some(act)
            }
            Action::OpenOverlay { dest, position, offset, anchor, .. } => {
                let mut act = node(self, dest, "OVERLAY")?;
                let size = self.doc.node(*dest).map_or([0.0, 0.0], |d| [d.width, d.height]);
                let offset = match anchor.offset(*position, self.trigger_size, size) {
                    Some(o) => Some(o),
                    None if *position == OverlayPosition::Manual => *offset,
                    None => None,
                };
                if let (Kv::Object(f), Some([x, y])) = (&mut act, offset) {
                    f.push(("overlayRelativePosition", o(vec![("x", fl(x)), ("y", fl(y))])));
                }
                Some(act)
            }
            Action::SwapOverlay { dest } => node(self, dest, "SWAP"),
            Action::ChangeTo { dest } => node(self, dest, "SWAP_STATE"),
            Action::ScrollTo { dest, offset } => {
                if *offset != [0.0, 0.0] {
                    self.warn("scroll-to offsets were left out");
                }
                node(self, dest, "SCROLL_TO")
            }
            Action::Back => plain("BACK"),
            Action::CloseOverlay => plain("CLOSE"),
            Action::OpenUrl { url, .. } => Some(o(vec![("connectionType", e("URL")), ("connectionURL", s(url))])),
            Action::SetVariable { variable, value, expr } => {
                let target = self.doc.variables.get_by_id(variable)?;
                let id = self.var_ids.get(variable.as_str())?;
                let data = match expr {
                    Some(expr) => self.expression(expr, expression_levels)?,
                    None => {
                        let literal = match target.kind {
                            VariableType::Boolean => VariableValue::Boolean(value.as_bool()?),
                            VariableType::Number => VariableValue::Number(value.as_f64()?),
                            VariableType::String => VariableValue::String(value.as_str()?.to_owned()),
                            VariableType::Color => VariableValue::Color(super::Color::parse_css(value.as_str()?)?),
                        };
                        literal_data(&literal)?
                    }
                };
                Some(o(vec![
                    ("connectionType", e("SET_VARIABLE")),
                    ("targetVariable", o(vec![("id", o(vec![("guid", guid(LAYERS, *id))]))])),
                    ("targetVariableData", data),
                ]))
            }
            Action::SetVariableMode { collection, mode } => {
                let set = self.set_ids.get(collection.as_str())?;
                let mode = self.mode_ids.get(&(collection.as_str(), mode.as_str()))?;
                Some(o(vec![
                    ("connectionType", e("SET_VARIABLE_MODE")),
                    ("targetVariableSetID", o(vec![("guid", guid(LAYERS, *set))])),
                    ("targetVariableModeID", guid(AUX, *mode)),
                ]))
            }
            Action::Conditional { blocks } if depth < MAX_ACTION_DEPTH => {
                let mut encoded = Vec::new();
                for block in blocks {
                    // Dropping an unreadable condition would turn this branch into an else.
                    let mut fields = Vec::new();
                    if let Some(condition) = &block.condition {
                        fields.push(("condition", self.expression(condition, expression_levels)?));
                    }
                    let actions = block.actions.iter().map(|action| self.action(action, it, depth + 1)).collect::<Option<Vec<_>>>()?;
                    fields.push(("actions", arr(actions)));
                    encoded.push(o(fields));
                }
                Some(o(vec![("connectionType", e("CONDITIONAL")), ("conditionalActions", arr(encoded))]))
            }
            Action::Conditional { .. } => None,
        }
    }

    /// Encode an expression within the remaining Kiwi operator nesting budget.
    fn expression(&self, expr: &Expr, levels: usize) -> Option<Kv<'static>> {
        match expr {
            Expr::Value(VariableValue::Alias(id)) => self.expression_alias(id),
            Expr::Value(literal) => literal_data(literal),
            Expr::Op { op, args } if levels > 0 && (1..=64).contains(&args.len()) => {
                let args = args.iter().map(|arg| self.expression(arg, levels - 1)).collect::<Option<Vec<_>>>()?;
                let (function, resolved) = match op {
                    ExprOp::Add => {
                        let string = args.iter().any(|arg| arg.get("resolvedDataType").and_then(Kv::str) == Some("STRING"));
                        ("ADDITION", if string { "STRING" } else { "FLOAT" })
                    }
                    ExprOp::Subtract => ("SUBTRACTION", "FLOAT"),
                    ExprOp::Multiply => ("MULTIPLY", "FLOAT"),
                    ExprOp::Divide => ("DIVIDE", "FLOAT"),
                    ExprOp::Negate => ("NEGATE", "FLOAT"),
                    ExprOp::Equals => ("EQUALS", "BOOLEAN"),
                    ExprOp::NotEqual => ("NOT_EQUAL", "BOOLEAN"),
                    ExprOp::Less => ("LESS_THAN", "BOOLEAN"),
                    ExprOp::LessOrEqual => ("LESS_THAN_OR_EQUAL", "BOOLEAN"),
                    ExprOp::Greater => ("GREATER_THAN", "BOOLEAN"),
                    ExprOp::GreaterOrEqual => ("GREATER_THAN_OR_EQUAL", "BOOLEAN"),
                    ExprOp::And => ("AND", "BOOLEAN"),
                    ExprOp::Or => ("OR", "BOOLEAN"),
                    ExprOp::Not => ("NOT", "BOOLEAN"),
                };
                Some(expression_data(function, resolved, args))
            }
            Expr::InMode { variable, mode } if levels > 0 => {
                let var = self.doc.variables.get_by_id(variable)?;
                let mode = self.mode_ids.get(&(var.collection.as_str(), mode.as_str()))?;
                let args = vec![self.expression_alias(variable)?, variable_data("STRING", "STRING", ("textValue", s(&format!("{AUX}:{mode}"))))];
                Some(expression_data("VAR_MODE_LOOKUP", super::common::resolved_type(var.kind), args))
            }
            Expr::Mode { .. } | Expr::Op { .. } | Expr::InMode { .. } => None,
        }
    }

    /// Reference an exported variable by GUID and include its resolved type.
    fn expression_alias(&self, variable: &str) -> Option<Kv<'static>> {
        let var = self.doc.variables.get_by_id(variable)?;
        let local = self.var_ids.get(variable)?;
        Some(variable_data("ALIAS", super::common::resolved_type(var.kind), ("alias", o(vec![("guid", guid(LAYERS, *local))]))))
    }

    /// An interaction's transition on one of its actions.
    fn transition(&mut self, it: &Interaction, f: &mut Fields) {
        let side_from = |d: Direction, l: &'static str, r: &'static str, t: &'static str, bo: &'static str| match d {
            // The direction names where the incoming screen moves: it comes from the other side.
            Direction::Right => l,
            Direction::Left => r,
            Direction::Bottom => t,
            Direction::Top => bo,
        };
        let side_to = |d: Direction, l: &'static str, r: &'static str, t: &'static str, bo: &'static str| match d {
            Direction::Left => l,
            Direction::Right => r,
            Direction::Top => t,
            Direction::Bottom => bo,
        };
        let ty = match it.transition {
            Transition::Instant => "INSTANT_TRANSITION",
            Transition::Dissolve => "DISSOLVE",
            Transition::SmartAnimate => "SMART_ANIMATE",
            Transition::SlideIn { direction } => side_from(direction, "SLIDE_FROM_LEFT", "SLIDE_FROM_RIGHT", "SLIDE_FROM_TOP", "SLIDE_FROM_BOTTOM"),
            Transition::Push { direction } => side_from(direction, "PUSH_FROM_LEFT", "PUSH_FROM_RIGHT", "PUSH_FROM_TOP", "PUSH_FROM_BOTTOM"),
            Transition::MoveIn { direction } => side_from(direction, "MOVE_FROM_LEFT", "MOVE_FROM_RIGHT", "MOVE_FROM_TOP", "MOVE_FROM_BOTTOM"),
            Transition::SlideOut { direction } => {
                side_to(direction, "SLIDE_OUT_TO_LEFT", "SLIDE_OUT_TO_RIGHT", "SLIDE_OUT_TO_TOP", "SLIDE_OUT_TO_BOTTOM")
            }
            Transition::MoveOut { direction } => side_to(direction, "MOVE_OUT_TO_LEFT", "MOVE_OUT_TO_RIGHT", "MOVE_OUT_TO_TOP", "MOVE_OUT_TO_BOTTOM"),
        };
        let easing = match it.easing {
            Easing::Linear => "LINEAR",
            Easing::EaseIn => "IN_CUBIC",
            Easing::EaseOut => "OUT_CUBIC",
            Easing::EaseInOut => "INOUT_CUBIC",
            Easing::EaseInBack => "IN_BACK_CUBIC",
            Easing::EaseOutBack => "OUT_BACK_CUBIC",
            Easing::EaseInOutBack => "INOUT_BACK_CUBIC",
            Easing::Gentle => "GENTLE_SPRING",
            Easing::CustomBezier { .. } => "CUSTOM_CUBIC",
            Easing::Quick | Easing::Bouncy | Easing::Slow | Easing::CustomSpring { .. } => {
                self.warn("spring animations other than Gentle were written as ease out");
                "OUT_CUBIC"
            }
        };
        f.extend([("transitionType", e(ty)), ("transitionDuration", fl(f64::from(it.duration_ms) / 1000.0)), ("easingType", e(easing))]);
        if let Easing::CustomBezier { x1, y1, x2, y2 } = it.easing {
            f.push(("easingFunction", arr(vec![fl(x1), fl(y1), fl(x2), fl(y2)])));
        }
    }
}

/// Wrap a literal, alias or expression in the Kiwi variable-data record.
fn variable_data(kind: &'static str, resolved: &'static str, value: (&'static str, Kv<'static>)) -> Kv<'static> {
    o(vec![("dataType", e(kind)), ("resolvedDataType", e(resolved)), ("value", o(vec![value]))])
}

/// Encode a finite, typed literal without variable references.
fn literal_data(value: &VariableValue) -> Option<Kv<'static>> {
    let (kind, field) = match value {
        VariableValue::Boolean(v) => ("BOOLEAN", ("boolValue", b(*v))),
        VariableValue::Number(v) if v.is_finite() => ("FLOAT", ("floatValue", fl(*v))),
        VariableValue::String(v) => ("STRING", ("textValue", s(&v.replace('\0', "")))),
        VariableValue::Color(v) => ("COLOR", ("colorValue", color(*v))),
        _ => return None,
    };
    Some(variable_data(kind, kind, field))
}

/// Encode an operator and its ordered arguments with their result type.
fn expression_data(function: &'static str, resolved: &'static str, args: Vec<Kv<'static>>) -> Kv<'static> {
    variable_data("EXPRESSION", resolved, ("expressionValue", o(vec![("expressionFunction", e(function)), ("expressionArguments", arr(args))])))
}
