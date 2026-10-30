//! Prototype commands: interactions, flows, settings, presenting.

use figcraft_doc::proto::{
    Action, DeviceRotation, Easing, Expr, Flow, Interaction, MAX_DELAY_MS, MAX_EXPR_DEPTH, OverlayPosition, PrototypeSettings, Transition, Trigger,
    walk_mut,
};
use figcraft_doc::{Document, NodeId, NodeKind, VariableValue, Variables};
use serde_json::{Value, json};

use super::{CommandSpec, ok};
use crate::params::{boolean, color, f64_of, get, id, num, parse, req_id, req_num, req_targets, string};
use crate::{Result, Session, bad, spec};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "proto.interaction.add",
            "Add interaction",
            "{id?, trigger? {type: onClick|onHover|onPress|onDrag|mouseEnter|mouseLeave|mouseDown|mouseUp|afterDelay {ms}|keyPress {key (a key or a combination: Shift+K, Ctrl+Alt+Delete; or a controller button: Gamepad A|B|X|Y|LB|RB|LT|RT|View|Menu|LS|RS|Up|Down|Left|Right|Home)}}, action {type: navigate {preserveScroll?}|back|scrollTo {offset? [x, y] (where the layer ends up from the top-left)}|openOverlay {position?, anchor? screen|triggerInside|triggerOutside (default screen; the trigger ones measure the position preset on the layer that opens the overlay: outside puts top presets above it and bottom presets below it, edges lined up), offset? [x, y] (position manual: the overlay's top-left from the top-left of the layer that opens it; without it, right below that layer), close_on_outside? (default true), background?}|swapOverlay|closeOverlay|openUrl {url}|changeTo|setVariable {variable, value | expression (text, e.g. \"count + 1\")}|setVariableMode {collection, mode}|conditional {blocks [{condition? | expression? (text, e.g. \"count >= 3\"), actions}]}, dest?}, actions? [..], dest? (without action: a click-navigate to it), transition? {type: instant|dissolve|smartAnimate|moveIn|moveOut|push|slideIn|slideOut, direction?: left|right|top|bottom}, durationMs? | duration_ms?, easing? linear|easeIn|easeOut|easeInOut|easeInBack|easeOutBack|easeInOutBack|gentle|quick|bouncy|slow|{customBezier {x1, y1, x2, y2}}|{customSpring {mass, stiffness, damping}}, delayMs? | delay_ms? (mouse enter, leave, down and up: how long after the gesture the actions run, up to 600000)} — or {id, dest} for a click-navigate. Returns {index}",
            true,
            add
        ),
        spec!("proto.interaction.clear", "Remove all interactions", "{ids?}", true, |s, p| {
            let ids = req_targets("proto.interaction.clear", s, p)?;
            let doc = s.doc_mut()?;
            for i in ids {
                doc.get_mut(i)?.interactions.clear();
            }
            ok()
        }),
        spec!("proto.interaction.remove", "Remove interaction", "{id?, index}", true, |s, p| {
            let cmd = "proto.interaction.remove";
            let k = req_num(cmd, p, "index")?;
            if k < 0.0 {
                return Err(bad(cmd, "`index` must be ≥ 0"));
            }
            let k = k as usize;
            let mut hit = false;
            for i in req_targets(cmd, s, p)? {
                let n = s.doc_mut()?.get_mut(i)?;
                if k < n.interactions.len() {
                    n.interactions.remove(k);
                    hit = true;
                }
            }
            if !hit {
                return Err(bad(cmd, format!("no interaction {k}")));
            }
            ok()
        }),
        spec!(
            "proto.interaction.set",
            "Edit interaction",
            "{id?, index, interaction? {..} (replaces it whole), trigger?, action? (first action), actions?, transition?, durationMs? | duration_ms?, easing?, delayMs? | delay_ms?}  patch fields on top of the current interaction (or of interaction)",
            true,
            set
        ),
        spec!("proto.flow.add", "Add flow starting point", "{start? (top-level frame id; default selection), name?}", true, |s, p| {
            let cmd = "proto.flow.add";
            let start = id(p, "start").or_else(|| s.selection().first().copied()).ok_or_else(|| bad(cmd, "select a top-level frame"))?;
            let page = s.page()?;
            let doc = s.doc_mut()?;
            if doc.top_level_frame(start) != Some(start) {
                return Err(bad(cmd, "a flow starts at a top-level frame"));
            }
            let flows = match &doc.get(page)?.kind {
                NodeKind::Page { flows, .. } => flows.clone(),
                _ => vec![],
            };
            if let Some(f) = flows.iter().find(|f| f.start == start) {
                return Ok(json!({"name": f.name, "start": start}));
            }
            let name = string(p, "name").filter(|n| !n.trim().is_empty()).unwrap_or_else(|| {
                let mut k = flows.len() + 1;
                while flows.iter().any(|f| f.name == format!("Flow {k}")) {
                    k += 1;
                }
                format!("Flow {k}")
            });
            if let NodeKind::Page { flows, .. } = &mut doc.get_mut(page)?.kind {
                flows.push(Flow { name: name.clone(), start, description: String::new() });
            }
            Ok(json!({"name": name, "start": start}))
        }),
        spec!("proto.flow.rename", "Rename flow", "{start, name?, description?}", true, |s, p| {
            let cmd = "proto.flow.rename";
            let start = req_id(cmd, p, "start")?;
            let name = string(p, "name").map(|n| n.trim().to_string());
            if name.as_deref() == Some("") {
                return Err(bad(cmd, "`name` can't be empty"));
            }
            let desc = string(p, "description");
            let page = s.page()?;
            let NodeKind::Page { flows, .. } = &mut s.doc_mut()?.get_mut(page)?.kind else { return Err(bad(cmd, "not a page")) };
            let f = flows.iter_mut().find(|f| f.start == start).ok_or_else(|| bad(cmd, "no flow starts at that frame"))?;
            if let Some(n) = name {
                f.name = n;
            }
            if let Some(d) = desc {
                f.description = d;
            }
            ok()
        }),
        spec!("proto.flow.remove", "Remove flow", "{start}", true, |s, p| {
            let start = req_id("proto.flow.remove", p, "start")?;
            let page = s.page()?;
            if let NodeKind::Page { flows, .. } = &mut s.doc_mut()?.get_mut(page)?.kind {
                flows.retain(|f| f.start != start);
            }
            ok()
        }),
        spec!(
            "proto.settings",
            "Prototype settings",
            "{device? (a device name, \"Custom size\" or \"None\"), background?, rotation? portrait|landscape (or 0|90; phones and tablets turn), color? (one of the device's colours by name), width?, height? (the custom size's screen, 1–10000 points; picks \"Custom size\"), responsive? bool (frames resize to the screen and their constraints and auto layout respond)}",
            true,
            settings
        ),
        spec!(
            "proto.present",
            "Present",
            "{start? (frame id; default: the selected frame, else the first flow, else the first frame)}",
            false,
            |s, p| { open_player(s, p, "proto.present", "present") }
        ),
        spec!(
            "proto.preview",
            "Open prototype preview",
            "{start? (frame id; default as proto.present)}  a floating preview pane over the canvas that follows edits live",
            false,
            |s, p| open_player(s, p, "proto.preview", "preview")
        ),
        spec!("proto.closePreview", "Close prototype preview", "{}", false, |s, _| {
            s.request_ui("preview.close", json!({}));
            ok()
        }),
        spec!(
            "proto.presentOptions",
            "Presentation options",
            "{scale? fit|fitWidth|fill|actual, deviceFrame? bool, hotspotHints? bool, inspect? bool}  how the presenter sizes the prototype (Fit scales down to fit, Fit width fills the width, Fill covers the window, Actual shows it at 100%), whether it draws the device, whether a click where nothing reacts flashes the hotspots (on by default) and whether the Inspect panel shows the properties of the layer under the pointer; kept for later presentations",
            false,
            |s, p| {
                let cmd = "proto.presentOptions";
                let mut data = serde_json::Map::new();
                if let Some(v) = p.get("scale").filter(|v| !v.is_null()) {
                    let scale = v.as_str().filter(|x| PRESENT_SCALES.contains(x)).ok_or_else(|| bad(cmd, "scale: fit|fitWidth|fill|actual"))?;
                    data.insert("scale".into(), json!(scale));
                }
                for key in ["deviceFrame", "hotspotHints", "inspect"] {
                    if let Some(v) = p.get(key).filter(|v| !v.is_null()) {
                        data.insert(key.into(), json!(v.as_bool().ok_or_else(|| bad(cmd, format!("{key}: true|false")))?));
                    }
                }
                if data.is_empty() {
                    return Err(bad(cmd, "nothing to set: scale, deviceFrame, hotspotHints or inspect"));
                }
                s.request_ui("present.options", Value::Object(data));
                ok()
            }
        ),
        spec!(
            "proto.linkNotice",
            "Notice before leaving for a link",
            "{show? (default true; false skips it)}  whether a presented prototype's Open link the viewer clicked (or fired by a key) first says that it leaves the prototype, with Open link and Cancel (a link a delay or hover fired always does); kept with the preferences → {shown}",
            false,
            |s, p| {
                let show = boolean(p, "show").unwrap_or(true);
                s.prefs.link_notice_skipped = !show;
                Ok(json!({"shown": show}))
            }
        ),
    ]
}

/// `proto.settings`: everything is checked before anything changes.
fn settings(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "proto.settings";
    let page = s.page()?;
    let bg = color(p, "background");
    let dev = string(p, "device");
    let rot = match p.get("rotation") {
        None | Some(Value::Null) => None,
        Some(v) => Some(rotation_of(v).ok_or_else(|| bad(cmd, "rotation: portrait|landscape"))?),
    };
    let side = |key: &str| -> Result<Option<f64>> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_f64().and_then(PrototypeSettings::custom_side).map(Some).ok_or_else(|| bad(cmd, format!("{key}: 1–10000 points"))),
        }
    };
    let (w, h) = (side("width")?, side("height")?);
    let responsive = match p.get("responsive") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| bad(cmd, "responsive: true|false"))?),
    };
    let col = match p.get("color") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or_else(|| bad(cmd, "color: one of the device's colour names"))?.trim().to_string()),
    };
    let NodeKind::Page { prototype, .. } = &s.doc()?.get(page)?.kind else { return Err(bad(cmd, "not a page")) };
    let mut next = prototype.clone();
    if let Some(c) = bg {
        next.background = c;
    }
    if let Some(d) = dev {
        next.device = d;
    }
    if let Some(r) = rot {
        next.rotation = r;
    }
    // A width or height is the custom size's: it picks "Custom size".
    if w.is_some() || h.is_some() {
        next.custom_size = [w.unwrap_or(next.custom_size[0]), h.unwrap_or(next.custom_size[1])];
        next.device = PrototypeSettings::CUSTOM_DEVICE.into();
    }
    if let Some(r) = responsive {
        next.responsive = r;
    }
    let colors = next.model().map_or(&[][..], |m| m.colors);
    match col {
        Some(c) if c.is_empty() => next.color.clear(),
        Some(c) => {
            let found = colors.iter().find(|x| x.name.eq_ignore_ascii_case(&c)).ok_or_else(|| {
                let names: Vec<&str> = colors.iter().map(|x| x.name).collect();
                bad(cmd, if names.is_empty() { "this device has no colours".to_string() } else { format!("color: {}", names.join("|")) })
            })?;
            next.color = found.name.into();
        }
        // Another device keeps the colour only if it comes in it.
        None if !colors.iter().any(|x| x.name == next.color) => next.color.clear(),
        None => {}
    }
    if next != *prototype
        && let NodeKind::Page { prototype, .. } = &mut s.doc_mut()?.get_mut(page)?.kind
    {
        *prototype = next;
    }
    ok()
}

/// Resize top-level `frame` to a screen of `(w, h)` as a responsive prototype does: its children
/// follow their constraints and auto layout flows again (fill layers stretch, text wraps), exactly
/// as when the frame is resized in the editor; an axis the frame hugs keeps hugging its content.
/// Used by the player on its own copy of the document.
pub fn respond(doc: &mut Document, frame: NodeId, (w, h): (f64, f64)) {
    if !(w.is_finite() && h.is_finite()) || doc.node(frame).is_none_or(|n| !n.kind.is_frame_like()) {
        return;
    }
    let (w, h) = (w.clamp(1.0, 100_000.0), h.clamp(1.0, 100_000.0));
    if doc.node(frame).is_some_and(|n| (n.width - w).abs() < 1e-9 && (n.height - h).abs() < 1e-9) {
        return;
    }
    doc.take_dirty();
    let before = doc.clone();
    if super::props::resize_node(doc, frame, w, h).is_ok() {
        crate::normalize(doc, &before);
    }
}

/// The presenter's scale options (`proto.presentOptions {scale}`).
pub const PRESENT_SCALES: [&str; 4] = ["fit", "fitWidth", "fill", "actual"];

/// A device rotation from a command value: `portrait` / `landscape`, or 0 / 90 degrees.
fn rotation_of(v: &Value) -> Option<DeviceRotation> {
    match v {
        Value::String(s) => DeviceRotation::parse(s),
        Value::Number(n) => n.as_f64().and_then(|a| DeviceRotation::parse(&format!("{a}"))),
        _ => None,
    }
}

/// Ask the UI to present (full screen or in the preview pane) from `start`, else the selected
/// top-level frame, the page's first flow or its first frame.
fn open_player(s: &mut Session, p: &Value, cmd: &str, ui: &str) -> Result<Value> {
    let ds = s.ds()?;
    let start = match id(p, "start") {
        // A layer inside a frame presents its top-level frame.
        Some(f) => ds.doc.top_level_frame(f).ok_or_else(|| bad(cmd, "`start` isn't a frame or inside one"))?,
        None => ds
            .selection
            .first()
            .and_then(|i| ds.doc.top_level_frame(*i))
            .or_else(|| match &ds.doc.node(ds.page)?.kind {
                NodeKind::Page { flows, .. } => flows.first().map(|f| f.start),
                _ => None,
            })
            .or_else(|| ds.doc.top_frames(ds.page).first().copied())
            .ok_or_else(|| bad(cmd, "no frame to present"))?,
    };
    s.request_ui(ui, json!({"start": start}));
    Ok(json!({"start": start}))
}

/// Every node an action points at must exist.
fn check_dests(cmd: &str, s: &Session, it: &Interaction) -> Result<()> {
    check_action_dests(cmd, s.doc()?, &it.actions)
}

/// Every node `actions` (and what they nest) point at must exist in `doc`.
pub(crate) fn check_action_dests(cmd: &str, doc: &Document, actions: &[Action]) -> Result<()> {
    for a in figcraft_doc::proto::walk(actions) {
        if let Some(d) = a.dest()
            && !doc.contains(d)
        {
            return Err(bad(cmd, format!("destination {d} doesn't exist")));
        }
    }
    Ok(())
}

/// Variables, collections and modes an expression names by name, stored by id.
pub(crate) fn canonical_expr(vars: &Variables, e: &mut Expr, depth: usize) {
    match e {
        Expr::Value(VariableValue::Alias(name)) => {
            if let Some(v) = vars.get(name) {
                *name = v.id.clone();
            }
        }
        Expr::Mode { collection } => {
            if let Some(c) = vars.collection(collection) {
                *collection = c.id.clone();
            }
        }
        Expr::InMode { variable, mode } => {
            if let Some(v) = vars.get(variable) {
                if let Some(m) = vars.collection_by_id(&v.collection).and_then(|c| c.find_mode(mode)) {
                    *mode = m.id.clone();
                }
                *variable = v.id.clone();
            }
        }
        Expr::Op { args, .. } if depth < MAX_EXPR_DEPTH => args.iter_mut().for_each(|a| canonical_expr(vars, a, depth + 1)),
        _ => {}
    }
}

fn duration(p: &Value) -> Option<u32> {
    num(p, "durationMs").or_else(|| num(p, "duration_ms")).map(|d| d.clamp(0.0, 60_000.0) as u32)
}

/// The delay of a mouse enter, leave, down or up trigger (`delayMs`), capped.
fn delay(cmd: &str, p: &Value) -> Result<Option<u32>> {
    match get(p, "delayMs").or_else(|| get(p, "delay_ms")) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match f64_of(v).filter(|d| d.is_finite() && *d >= 0.0) {
            Some(d) => Ok(Some(d.min(f64::from(MAX_DELAY_MS)).round() as u32)),
            None => Err(bad(cmd, "delayMs: milliseconds, 0 or more")),
        },
    }
}

/// A delay only where it applies (mouse enter, leave, down and up), and a manual offset only on a
/// manual overlay, within what the player places.
fn settle(it: &mut Interaction) {
    if it.delay_secs() == 0.0 {
        it.delay_ms = 0;
    }
    it.delay_ms = it.delay_ms.min(MAX_DELAY_MS);
    walk_mut(&mut it.actions, &mut |a| {
        if let Action::OpenOverlay { position, offset, .. } = a {
            *offset =
                offset.filter(|o| *position == OverlayPosition::Manual && o.iter().all(|c| c.is_finite())).map(|o| o.map(|c| c.clamp(-1e5, 1e5)));
        }
    });
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "proto.interaction.add";
    let p = &super::proto_actions::with_expressions(cmd, &s.doc()?.variables, p)?;
    let it = if let Some(d) = id(p, "dest")
        && get(p, "action").is_none()
    {
        Interaction {
            trigger: parse(cmd, p, "trigger")?.unwrap_or_default(),
            actions: vec![Action::Navigate { dest: d, preserve_scroll: false }],
            transition: parse(cmd, p, "transition")?.unwrap_or_default(),
            duration_ms: duration(p).unwrap_or(300),
            easing: parse(cmd, p, "easing")?.unwrap_or_default(),
            delay_ms: 0,
        }
    } else {
        let trigger: Trigger = parse(cmd, p, "trigger")?.unwrap_or_default();
        let actions: Vec<Action> = match parse::<Vec<Action>>(cmd, p, "actions")? {
            Some(a) if !a.is_empty() => a,
            _ => vec![parse(cmd, p, "action")?.ok_or_else(|| bad(cmd, "`action` required"))?],
        };
        let transition: Transition = parse(cmd, p, "transition")?.unwrap_or_default();
        let easing: Easing = parse(cmd, p, "easing")?.unwrap_or_default();
        Interaction { trigger, actions, transition, duration_ms: duration(p).unwrap_or(300), easing, delay_ms: 0 }
    };
    let mut it = it;
    it.delay_ms = delay(cmd, p)?.unwrap_or(0);
    settle(&mut it);
    check_dests(cmd, s, &it)?;
    let mut index = 0;
    for i in req_targets(cmd, s, p)? {
        let n = s.doc_mut()?.get_mut(i)?;
        n.interactions.push(it.clone());
        index = n.interactions.len() - 1;
    }
    Ok(json!({"index": index}))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "proto.interaction.set";
    let p = &super::proto_actions::with_expressions(cmd, &s.doc()?.variables, p)?;
    let k = req_num(cmd, p, "index")?;
    if k < 0.0 {
        return Err(bad(cmd, "`index` must be ≥ 0"));
    }
    let k = k as usize;
    let whole: Option<Interaction> = parse(cmd, p, "interaction")?;
    let trigger: Option<Trigger> = parse(cmd, p, "trigger")?;
    let action: Option<Action> = parse(cmd, p, "action")?;
    let actions: Option<Vec<Action>> = parse(cmd, p, "actions")?;
    let transition: Option<Transition> = parse(cmd, p, "transition")?;
    let easing: Option<Easing> = parse(cmd, p, "easing")?;
    let dur = duration(p);
    let wait = delay(cmd, p)?;
    let mut hit = false;
    for i in req_targets(cmd, s, p)? {
        let Some(cur) = s.doc()?.get(i)?.interactions.get(k).cloned() else { continue };
        let mut it = whole.clone().unwrap_or(cur);
        if let Some(t) = &trigger {
            it.trigger = t.clone();
        }
        if let Some(a) = &actions {
            it.actions = a.clone();
        }
        if let Some(a) = &action {
            match it.actions.first_mut() {
                Some(slot) => *slot = a.clone(),
                None => it.actions.push(a.clone()),
            }
        }
        if let Some(t) = transition {
            it.transition = t;
        }
        if let Some(e) = easing {
            it.easing = e;
        }
        if let Some(d) = dur {
            it.duration_ms = d;
        }
        if let Some(d) = wait {
            it.delay_ms = d;
        }
        settle(&mut it);
        if it.actions.is_empty() {
            return Err(bad(cmd, "an interaction needs an action"));
        }
        check_dests(cmd, s, &it)?;
        if let Some(slot) = s.doc_mut()?.get_mut(i)?.interactions.get_mut(k) {
            *slot = it;
            hit = true;
        }
    }
    if !hit {
        return Err(bad(cmd, format!("no interaction {k}")));
    }
    ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::id_of;
    use figcraft_doc::proto::{Direction, Overflow, ScrollPosition};

    fn frame(s: &mut Session, x: f64) -> NodeId {
        let r = s.execute("node.create", &json!({"type": "frame", "x": x, "y": 0, "width": 100, "height": 200})).unwrap();
        id_of(&r["id"]).unwrap()
    }

    fn page_flows(s: &Session) -> Vec<Flow> {
        let ds = s.ds().unwrap();
        match &ds.doc.node(ds.page).unwrap().kind {
            NodeKind::Page { flows, .. } => flows.clone(),
            _ => vec![],
        }
    }

    #[test]
    fn add_set_patch_remove() {
        let mut s = Session::new();
        let a = frame(&mut s, 0.0);
        let b = frame(&mut s, 200.0);
        let r = s.execute("proto.interaction.add", &json!({"id": a, "dest": b})).unwrap();
        assert_eq!(r["index"], 0);
        let r = s
            .execute(
                "proto.interaction.add",
                &json!({"id": a, "trigger": {"type": "keyPress", "key": "Space"}, "action": {"type": "back"}, "transition": {"type": "moveIn"}, "durationMs": 450, "easing": "bouncy"}),
            )
            .unwrap();
        assert_eq!(r["index"], 1);
        let it = s.doc().unwrap().node(a).unwrap().interactions[1].clone();
        assert_eq!(it.trigger, Trigger::KeyPress { key: "Space".into() });
        assert_eq!(it.transition, Transition::MoveIn { direction: Direction::Left });
        assert_eq!(it.duration_ms, 450);
        assert_eq!(it.easing, Easing::Bouncy);
        // Patch only the transition and easing; the action stays.
        s.execute("proto.interaction.set", &json!({"id": a, "index": 0, "transition": {"type": "push", "direction": "top"}, "easing": "linear"}))
            .unwrap();
        let it = s.doc().unwrap().node(a).unwrap().interactions[0].clone();
        assert_eq!(it.actions, vec![Action::Navigate { dest: b, preserve_scroll: false }]);
        assert_eq!(it.transition, Transition::Push { direction: Direction::Top });
        assert_eq!(it.easing, Easing::Linear);
        // Overlay action with settings; camelCase accepted.
        s.execute(
            "proto.interaction.set",
            &json!({"id": a, "index": 0, "action": {"type": "openOverlay", "dest": b, "position": "bottomCenter", "closeOnOutside": true, "background": "#00000040"}}),
        )
        .unwrap();
        let it = s.doc().unwrap().node(a).unwrap().interactions[0].clone();
        let Action::OpenOverlay { position, close_on_outside, background, .. } = &it.actions[0] else { panic!("{it:?}") };
        assert_eq!(*position, OverlayPosition::BottomCenter);
        assert!(*close_on_outside);
        assert!(background.is_some());
        // Whole replacement round-trips through serde.
        let mut whole = it.clone();
        whole.trigger = Trigger::AfterDelay { ms: 800 };
        s.execute("proto.interaction.set", &json!({"id": a, "index": 0, "interaction": whole})).unwrap();
        assert_eq!(s.doc().unwrap().node(a).unwrap().interactions[0].trigger, Trigger::AfterDelay { ms: 800 });
        // Errors leave the document untouched.
        assert!(s.execute("proto.interaction.set", &json!({"id": a, "index": 9, "easing": "linear"})).is_err());
        assert!(s.execute("proto.interaction.set", &json!({"id": a, "index": 0, "action": {"type": "navigate", "dest": 99999}})).is_err());
        assert!(s.execute("proto.interaction.add", &json!({"id": a, "action": {"type": "navigate", "dest": 99999}})).is_err());
        assert!(s.execute("proto.interaction.set", &json!({"id": a, "index": 0, "actions": []})).is_err());
        assert!(s.execute("proto.interaction.remove", &json!({"id": a, "index": -1})).is_err());
        assert_eq!(s.doc().unwrap().node(a).unwrap().interactions.len(), 2);
        s.execute("proto.interaction.remove", &json!({"id": a, "index": 0})).unwrap();
        assert_eq!(s.doc().unwrap().node(a).unwrap().interactions.len(), 1);
        assert!(s.undo().unwrap());
        assert_eq!(s.doc().unwrap().node(a).unwrap().interactions.len(), 2);
    }

    #[test]
    fn flows_add_rename_remove() {
        let mut s = Session::new();
        let a = frame(&mut s, 0.0);
        let b = frame(&mut s, 200.0);
        let r = s.execute("proto.flow.add", &json!({"start": a})).unwrap();
        assert_eq!(r["name"], "Flow 1");
        // Adding again keeps the existing flow.
        s.execute("proto.flow.add", &json!({"start": a, "name": "Other"})).unwrap();
        assert_eq!(page_flows(&s).len(), 1);
        s.execute("proto.flow.add", &json!({"start": b})).unwrap();
        s.execute("proto.flow.rename", &json!({"start": b, "name": "Checkout", "description": "Pay"})).unwrap();
        let flows = page_flows(&s);
        assert_eq!(flows.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Flow 1", "Checkout"]);
        assert_eq!(flows[1].description, "Pay");
        assert!(s.execute("proto.flow.rename", &json!({"start": b, "name": "  "})).is_err());
        assert!(s.execute("proto.flow.rename", &json!({"start": 4242, "name": "x"})).is_err());
        // Only top-level frames start flows.
        let r = s.execute("node.create", &json!({"type": "rectangle", "parent": a, "x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
        let child = id_of(&r["id"]).unwrap();
        assert!(s.execute("proto.flow.add", &json!({"start": child})).is_err());
        s.execute("proto.flow.remove", &json!({"start": a})).unwrap();
        assert_eq!(page_flows(&s).len(), 1);
        // A new flow gets an unused default name.
        let r = s.execute("proto.flow.add", &json!({"start": a})).unwrap();
        assert_eq!(r["name"], "Flow 2");
    }

    #[test]
    fn present_and_preview_pick_a_top_level_frame() {
        let mut s = Session::new();
        let a = frame(&mut s, 0.0);
        let b = frame(&mut s, 200.0);
        let r = s.execute("node.create", &json!({"type": "rectangle", "parent": b, "x": 205, "y": 5, "width": 10, "height": 10})).unwrap();
        let child = id_of(&r["id"]).unwrap();
        let last = |s: &mut Session| {
            let r = s.ui_requests.pop().unwrap();
            s.ui_requests.clear();
            (r.kind, id_of(&r.data["start"]))
        };
        // A layer inside a frame previews its frame; the selection is the default.
        assert_eq!(s.execute("proto.preview", &json!({"start": child})).unwrap()["start"], json!(b));
        assert_eq!(last(&mut s), ("preview".to_string(), Some(b)));
        s.execute("select.set", &json!({"ids": [a]})).unwrap();
        s.execute("proto.present", &json!({})).unwrap();
        assert_eq!(last(&mut s), ("present".to_string(), Some(a)));
        // A start that isn't in a frame is an error, not a silent fallback.
        assert!(s.execute("proto.preview", &json!({"start": 987_654})).is_err());
        s.execute("proto.closePreview", &json!({})).unwrap();
        assert_eq!(last(&mut s).0, "preview.close");
    }

    #[test]
    fn navigate_can_preserve_scroll() {
        let mut s = Session::new();
        let a = frame(&mut s, 0.0);
        let b = frame(&mut s, 200.0);
        s.execute("proto.interaction.add", &json!({"id": a, "action": {"type": "navigate", "dest": b, "preserveScroll": true}})).unwrap();
        let it = s.doc().unwrap().node(a).unwrap().interactions[0].clone();
        assert_eq!(it.actions, vec![Action::Navigate { dest: b, preserve_scroll: true }]);
        // Off is the default and isn't written out.
        let off = serde_json::to_value(Action::Navigate { dest: b, preserve_scroll: false }).unwrap();
        assert!(off.get("preserve_scroll").is_none(), "{off}");
    }

    #[test]
    fn scroll_props_via_node_set() {
        let mut s = Session::new();
        let a = frame(&mut s, 0.0);
        s.execute("node.set", &json!({"id": a, "props": {"overflow": "vertical", "scrollPosition": "fixed"}})).unwrap();
        let n = s.doc().unwrap().node(a).unwrap().clone();
        assert_eq!(n.overflow, Overflow::Vertical);
        assert_eq!(n.scroll_position, ScrollPosition::Fixed);
    }
}
