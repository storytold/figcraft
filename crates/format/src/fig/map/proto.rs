use super::paint::color;
use super::variables::variable_data;
use super::{Kv, MAX_ACTION_DEPTH, Mapper, Obj, Value, View, copy_str, guid, json, num, ref_guid};

impl<'a> Mapper<'a> {
    pub(super) fn prototype(&mut self, o: &mut Obj, v: &View<'a>) {
        let mut list = Vec::new();
        for it in v.list("prototypeInteractions") {
            if it.get("isDeleted").and_then(Kv::bool) == Some(true) {
                continue;
            }
            if let Some(i) = self.interaction(it) {
                list.push(i);
            }
        }
        if !list.is_empty() {
            o.insert("interactions".into(), Value::Array(list));
        }
        let overflow = match v.str("scrollDirection") {
            Some("HORIZONTAL") => Some("HORIZONTAL_SCROLLING"),
            Some("VERTICAL") => Some("VERTICAL_SCROLLING"),
            Some("BOTH") => Some("HORIZONTAL_AND_VERTICAL_SCROLLING"),
            _ => None,
        };
        if let Some(d) = overflow {
            o.insert("overflowDirection".into(), json!(d));
        }
        let behavior = match v.str("scrollBehavior") {
            Some("FIXED_WHEN_CHILD_OF_SCROLLING_FRAME") => Some("FIXED"),
            Some("STICKY_SCROLLS") => Some("STICKY_SCROLLS"),
            _ => None,
        };
        if let Some(b) = behavior {
            o.insert("scrollBehavior".into(), json!(b));
        }
        copy_str(o, v, "overlayPositionType", "overlayPositionType");
        copy_str(o, v, "overlayBackgroundInteraction", "overlayBackgroundInteraction");
        if let Some(a) = v.get("overlayBackgroundAppearance") {
            let ty = a.get("backgroundType").and_then(Kv::str).unwrap_or("NONE");
            o.insert("overlayBackground".into(), json!({"type": ty, "color": a.get("backgroundColor").map_or(Value::Null, color)}));
        }
    }

    fn interaction(&mut self, it: &Kv) -> Option<Value> {
        let ev = it.get("event");
        let ty = match ev.and_then(|e| e.get("interactionType")).and_then(Kv::str).unwrap_or("ON_CLICK") {
            "DRAG" => "ON_DRAG",
            "MOUSE_IN" => "MOUSE_ENTER",
            "MOUSE_OUT" => "MOUSE_LEAVE",
            t => t,
        };
        let mut trigger = json!({"type": ty});
        if let Some(t) = ev.and_then(|e| num(e, "transitionTimeout")) {
            // Seconds as 32-bit floats: whole milliseconds.
            trigger["timeout"] = json!((t * 1000.0).round());
        }
        if let Some(k) = ev.and_then(|e| e.get("keyTrigger")) {
            let codes: Vec<f64> = k.get("keyCodes").map(Kv::list).unwrap_or_default().iter().filter_map(Kv::f64).collect();
            trigger["keyCodes"] = json!(codes);
            if let Some(d) = k.get("triggerDevice").and_then(Kv::str) {
                trigger["device"] = json!(if d.contains("KEYBOARD") { "KEYBOARD" } else { d });
            }
        }
        let actions: Vec<Value> = it.get("actions").map(Kv::list).unwrap_or_default().iter().filter_map(|a| self.action(a, 0)).collect();
        (!actions.is_empty()).then(|| json!({"trigger": trigger, "actions": actions}))
    }

    /// One prototype action in the REST shape (`depth`: conditionals it's nested in).
    fn action(&mut self, a: &Kv, depth: usize) -> Option<Value> {
        Some(match a.get("connectionType").and_then(Kv::str) {
            Some("INTERNAL_NODE") | None => {
                let dest = a.get("transitionNodeID").and_then(guid)?;
                let nav = match a.get("navigationType").and_then(Kv::str) {
                    Some("SWAP_STATE") => "CHANGE_TO",
                    Some(n) => n,
                    None => "NAVIGATE",
                };
                let mut act = json!({"type": "NODE", "destinationId": dest, "navigation": nav});
                if let Some(t) = transition(a) {
                    act["transition"] = t;
                }
                if let Some(b) = a.get("transitionPreserveScroll").and_then(Kv::bool) {
                    act["preserveScrollPosition"] = json!(b);
                }
                if let Some(p) = a.get("overlayRelativePosition") {
                    act["overlayRelativePosition"] = json!({"x": num(p, "x"), "y": num(p, "y")});
                }
                act
            }
            Some("URL") => json!({"type": "URL", "url": a.get("connectionURL").and_then(Kv::str)?}),
            Some("BACK") => json!({"type": "BACK"}),
            Some("CLOSE") => json!({"type": "CLOSE"}),
            Some("NONE") => return None,
            Some("SET_VARIABLE") => {
                let var = a.get("targetVariable").and_then(|t| t.get("id")).and_then(ref_guid).or_else(|| a.get("targetVariableID").and_then(guid));
                // Missing parts are left out: the REST importer skips the action with a warning.
                let mut act = json!({"type": "SET_VARIABLE", "variableId": var});
                if let Some(d) = a.get("targetVariableData").and_then(|d| variable_data(d, 0)) {
                    act["variableValue"] = d;
                }
                act
            }
            Some("SET_VARIABLE_MODE") => json!({
                "type": "SET_VARIABLE_MODE",
                "variableCollectionId": a.get("targetVariableSetID").and_then(ref_guid),
                "variableModeId": a.get("targetVariableModeID").and_then(guid),
            }),
            Some("CONDITIONAL") if depth < MAX_ACTION_DEPTH => {
                let mut blocks = Vec::new();
                for b in a.get("conditionalActions").map(Kv::list).unwrap_or_default() {
                    let actions: Vec<Value> =
                        b.get("actions").map(Kv::list).unwrap_or_default().iter().filter_map(|x| self.action(x, depth + 1)).collect();
                    let mut block = json!({"actions": actions});
                    // A condition that can't be read must not turn the block into an "else".
                    if let Some(c) = b.get("condition") {
                        block["condition"] = variable_data(c, 0).unwrap_or_else(|| json!({}));
                    }
                    blocks.push(block);
                }
                json!({"type": "CONDITIONAL", "conditionalBlocks": blocks})
            }
            Some(other) => {
                self.warn(format!("{} prototype actions are not imported from .fig files yet", other.to_ascii_lowercase().replace('_', " ")));
                return None;
            }
        })
    }
}

/// A prototype action's transition (`None`: instant).
fn transition(a: &Kv) -> Option<Value> {
    let ty = a.get("transitionType").and_then(Kv::str)?;
    // Directions name where the incoming screen moves (SLIDE_FROM_LEFT moves right).
    let (kind, dir) = match ty {
        "INSTANT_TRANSITION" => return None,
        "DISSOLVE" | "FADE" => ("DISSOLVE", None),
        "SMART_ANIMATE" | "MAGIC_MOVE" => ("SMART_ANIMATE", None),
        "SCROLL_ANIMATE" => ("SCROLL_ANIMATE", None),
        t => {
            let prefixes = [
                ("SLIDE_FROM_", "SLIDE_IN"),
                ("PUSH_FROM_", "PUSH"),
                ("MOVE_FROM_", "MOVE_IN"),
                ("SLIDE_OUT_TO_", "SLIDE_OUT"),
                ("MOVE_OUT_TO_", "MOVE_OUT"),
            ];
            let (kind, side) = prefixes.iter().find_map(|(p, k)| t.strip_prefix(p).map(|r| (*k, r)))?;
            let incoming = matches!(kind, "SLIDE_IN" | "PUSH" | "MOVE_IN");
            let dir = match (side, incoming) {
                ("LEFT", true) | ("RIGHT", false) => "RIGHT",
                ("RIGHT", true) | ("LEFT", false) => "LEFT",
                ("TOP", true) | ("BOTTOM", false) => "BOTTOM",
                _ => "TOP",
            };
            (kind, Some(dir))
        }
    };
    let easing = match a.get("easingType").and_then(Kv::str) {
        Some("IN_CUBIC" | "EASE_IN") => "EASE_IN",
        Some("INOUT_CUBIC") => "EASE_IN_AND_OUT",
        Some("LINEAR") => "LINEAR",
        Some("IN_BACK_CUBIC") => "EASE_IN_BACK",
        Some("OUT_BACK_CUBIC") => "EASE_OUT_BACK",
        Some("INOUT_BACK_CUBIC") => "EASE_IN_AND_OUT_BACK",
        Some("GENTLE_SPRING") => "GENTLE",
        Some("CUSTOM_CUBIC") => "CUSTOM_CUBIC_BEZIER",
        _ => "EASE_OUT",
    };
    let mut e = json!({"type": easing});
    let f: Vec<f64> = a.get("easingFunction").map(Kv::list).unwrap_or_default().iter().filter_map(Kv::f64).collect();
    if let (true, [x1, y1, x2, y2, ..]) = (easing == "CUSTOM_CUBIC_BEZIER", f.as_slice()) {
        e["easingFunctionCubicBezier"] = json!({"x1": x1, "y1": y1, "x2": x2, "y2": y2});
    }
    let mut t = json!({"type": kind, "duration": (num(a, "transitionDuration").unwrap_or(0.3) * 1000.0).round(), "easing": e});
    if let Some(d) = dir {
        t["direction"] = json!(d);
    }
    Some(t)
}
