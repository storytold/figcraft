use super::common::key_codes;
use super::{
    Action, Direction, Easing, Exporter, Fields, Interaction, Kv, Node, NodeId, Overflow, OverlayPosition, ScrollPosition, Transition, Trigger, arr,
    b, color, e, fl, int, o, position, s,
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
        let actions: Vec<Kv> = it.actions.iter().filter_map(|a| self.action(a, it)).collect();
        (!actions.is_empty()).then(|| o(vec![("event", o(event)), ("actions", arr(actions))]))
    }

    fn action(&mut self, a: &Action, it: &Interaction) -> Option<Kv<'static>> {
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
            Action::SetVariable { .. } | Action::SetVariableMode { .. } | Action::Conditional { .. } => {
                self.warn("variable and conditional prototype actions were left out");
                None
            }
        }
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
