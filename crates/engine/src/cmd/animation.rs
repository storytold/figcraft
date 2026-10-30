//! Animation commands: keyframes, clips, a layer's motion and its timeline settings.
//!
//! The playback itself lives in `figcraft_doc::anim`; these commands are the one way the editor, the
//! control channel and MCP change a document's animation, so an edit is an undo step, a read-only
//! document refuses it, and every change is normalized the same way.
//!
//! A drag runs its command on every frame inside one interaction
//! ([`Session::begin_interaction`](crate::Session)): the interaction records the snapshot taken at
//! the press and the frames record nothing, so the whole gesture stays one step.

use super::CommandSpec;
use crate::params::{boolean, num, opt_id, parse, req_id, req_num, req_string, req_targets, string};
use crate::{EngineError, Result, Session, bad, spec};
use figcraft_doc::anim::motion::{self, AnimProperty, Clip, ClipKind, ClipMode, Ease, NodeMotion};
use serde_json::{Value, json};

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "motion.keyframe",
            "Keyframe",
            "{ids?, property?, time?, value?, all?} (adds a keyframe at `time`: `property`'s value there, `value` when given, or every track the layer already animates with `all`)",
            true,
            keyframe_add
        ),
        spec!(
            "motion.keyframeMove",
            "Move keyframe",
            "{node, property, time, to} (the keyframe at `time` goes to `to`, kept between its neighbours; returns the time it landed on)",
            true,
            keyframe_move
        ),
        spec!("motion.keyframeDelete", "Delete keyframe", "{node, property, time}", true, keyframe_delete),
        spec!(
            "motion.keyframeEase",
            "Ease keyframe",
            "{node, property, time, ease} (the easing of the segment that starts at the keyframe)",
            true,
            keyframe_ease
        ),
        spec!(
            "motion.clip",
            "Add animation",
            "{ids?, kind, mode?, time?} (a clip of `kind` on the selected layers, starting at `time`; returns each layer's clip id)",
            true,
            clip_add
        ),
        spec!("motion.clipUpdate", "Edit animation clip", "{node, id, clip} (the whole clip, as the editor holds it)", true, clip_update),
        spec!("motion.clipDelete", "Delete animation clip", "{node, id}", true, clip_delete),
        spec!("motion.track", "Show/hide track", "{node, property, visible?} (without `visible`, the track flips)", true, track_visible),
        spec!(
            "motion.replace",
            "Move animation",
            "{node, motion} (a layer's whole motion: a timeline drag that slides or stretches it computes the result and sends it here)",
            true,
            motion_replace
        ),
        spec!(
            "motion.timeline",
            "Motion timeline",
            "{page?, duration?, fps?, loop?} (a page's timeline; a page back at the defaults leaves the document)",
            true,
            motion_timeline
        ),
    ]
}

/// The property a name picks (`Opacity`, `Position`, `Width`…), case-insensitively.
fn property_of(name: &str) -> Option<AnimProperty> {
    AnimProperty::ALL.iter().copied().find(|p| p.name().eq_ignore_ascii_case(name))
}

fn prop_param(cmd: &str, p: &Value) -> Result<AnimProperty> {
    let name = req_string(cmd, p, "property")?;
    property_of(&name).ok_or_else(|| {
        let names: Vec<&str> = AnimProperty::ALL.iter().map(AnimProperty::name).collect();
        bad(cmd, format!("`property` must be one of {}", names.join(", ")))
    })
}

fn kind_of(name: &str) -> Option<ClipKind> {
    ClipKind::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(name))
}

/// What a property's value looks like, for an error that says what to send instead.
fn shape(kind: motion::ValueKind) -> &'static str {
    match kind {
        motion::ValueKind::Scalar => "a single number",
        motion::ValueKind::Vec2 => "a pair of numbers",
        motion::ValueKind::Color => "a colour",
    }
}

fn mode_of(name: &str) -> Option<ClipMode> {
    match name.trim().to_ascii_lowercase().as_str() {
        "in" => Some(ClipMode::In),
        "out" => Some(ClipMode::Out),
        "custom" => Some(ClipMode::Custom),
        _ => None,
    }
}

/// The layer's motion, or an error naming the layer that isn't there.
fn motion_mut(doc: &mut figcraft_doc::Document, node: figcraft_doc::NodeId) -> Result<&mut NodeMotion> {
    doc.node_mut(node).map(|n| &mut n.motion).ok_or(EngineError::NoNode(node))
}

fn keyframe_add(s: &mut Session, p: &Value) -> Result<Value> {
    let time = num(p, "time").unwrap_or(0.0).max(0.0) as u32;
    let value: Option<motion::AnimValue> = parse("motion.keyframe", p, "value")?;
    let every_track = boolean(p, "all").unwrap_or(false);
    let property = match string(p, "property") {
        Some(_) => Some(prop_param("motion.keyframe", p)?),
        None => None,
    };
    if property.is_none() && !every_track {
        return Err(bad("motion.keyframe", "`property` is required (or `all`, for every track the layer already animates)"));
    }
    let ids = req_targets("motion.keyframe", s, p)?;
    // Every value has to fit the property it keys before anything is written. A scalar written into
    // `Position` would replace a two-part key with a one-part one, and `normalize` would then drop
    // that key: the command would report an addition after deleting animation data.
    if let Some(v) = &value {
        let doc = s.doc()?;
        for id in &ids {
            let props: Vec<AnimProperty> =
                property.map_or_else(|| doc.node(*id).map(|n| n.motion.tracks.iter().map(|t| t.property).collect()).unwrap_or_default(), |p| vec![p]);
            if let Some(prop) = props.iter().find(|prop| !prop.accepts(v)) {
                return Err(bad("motion.keyframe", format!("`value` does not fit `{}`: it takes {}", prop.name(), shape(prop.kind()))));
            }
        }
    }
    let doc = s.doc_mut()?;
    let mut added = 0;
    for id in &ids {
        let props: Vec<AnimProperty> =
            property.map_or_else(|| doc.node(*id).map(|n| n.motion.tracks.iter().map(|t| t.property).collect()).unwrap_or_default(), |p| vec![p]);
        let mut keys: Vec<(AnimProperty, motion::AnimValue)> = Vec::new();
        for prop in props {
            let v = value.or_else(|| doc.node(*id).and_then(|n| motion::keyframed_value_at(n, prop, time as f64)));
            if let Some(v) = v {
                keys.push((prop, v));
            }
        }
        if let Some(n) = doc.node_mut(*id) {
            for (prop, v) in keys {
                n.motion.ensure_track(prop).set_key(time, v, None);
                added += 1;
            }
            n.motion.normalize();
        }
    }
    doc.mark_dirty(ids);
    Ok(json!({"added": added, "time": time}))
}

fn keyframe_move(s: &mut Session, p: &Value) -> Result<Value> {
    let node = req_id("motion.keyframeMove", p, "node")?;
    let prop = prop_param("motion.keyframeMove", p)?;
    let from = req_num("motion.keyframeMove", p, "time")?.max(0.0) as u32;
    let to = req_num("motion.keyframeMove", p, "to")?.max(0.0) as u32;
    let doc = s.doc_mut()?;
    let m = motion_mut(doc, node)?;
    let mut landed = from;
    if let Some(tr) = m.track_mut(prop) {
        // A keyframe stays between its neighbours: two keys at one time would leave a key the
        // editor cannot address by time.
        let lo = tr.prev_key_time(from).map_or(0, |p| p.saturating_add(1));
        let hi = tr.next_key_time(from).map_or(u32::MAX, |p| p.saturating_sub(1));
        let time = to.clamp(lo, hi);
        if let Some(key) = tr.key_at(from).copied()
            && let Some(idx) = tr.key_index(from)
        {
            tr.keys[idx] = motion::Keyframe { time_ms: time, ..key };
            landed = time;
        }
    }
    m.normalize();
    doc.mark_dirty([node]);
    Ok(json!({"time": landed}))
}

fn keyframe_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let node = req_id("motion.keyframeDelete", p, "node")?;
    let prop = prop_param("motion.keyframeDelete", p)?;
    let time = req_num("motion.keyframeDelete", p, "time")?.max(0.0) as u32;
    let doc = s.doc_mut()?;
    let m = motion_mut(doc, node)?;
    let mut removed = false;
    if let Some(tr) = m.track_mut(prop) {
        removed = tr.key_at(time).is_some();
        tr.remove_key(time);
    }
    m.normalize();
    doc.mark_dirty([node]);
    Ok(json!({"removed": removed}))
}

fn keyframe_ease(s: &mut Session, p: &Value) -> Result<Value> {
    let node = req_id("motion.keyframeEase", p, "node")?;
    let prop = prop_param("motion.keyframeEase", p)?;
    let time = req_num("motion.keyframeEase", p, "time")?.max(0.0) as u32;
    let ease = parse::<Ease>("motion.keyframeEase", p, "ease")?.ok_or_else(|| bad("motion.keyframeEase", "`ease` is required"))?;
    let ease = ease.sanitized();
    let doc = s.doc_mut()?;
    let m = motion_mut(doc, node)?;
    let mut eased = false;
    if let Some(tr) = m.track_mut(prop)
        && let Some(idx) = tr.key_index(time)
    {
        tr.keys[idx].easing = ease;
        eased = true;
    }
    m.normalize();
    doc.mark_dirty([node]);
    Ok(json!({"eased": eased}))
}

fn clip_add(s: &mut Session, p: &Value) -> Result<Value> {
    let name = req_string("motion.clip", p, "kind")?;
    let kind = kind_of(&name).ok_or_else(|| {
        let names: Vec<&str> = ClipKind::ALL.iter().map(ClipKind::name).collect();
        bad("motion.clip", format!("`kind` must be one of {}", names.join(", ")))
    })?;
    let mode = match string(p, "mode") {
        Some(n) => mode_of(&n).ok_or_else(|| bad("motion.clip", "`mode` must be In, Out or Custom"))?,
        None => ClipMode::In,
    };
    let time = num(p, "time").unwrap_or(0.0).max(0.0) as u32;
    let ids = req_targets("motion.clip", s, p)?;
    let doc = s.doc_mut()?;
    let mut clips = Vec::new();
    for id in &ids {
        let Some(n) = doc.node_mut(*id) else { continue };
        let mut c = Clip::new(kind, mode);
        c.id = n.motion.next_clip_id();
        c.delay_ms = time;
        clips.push(json!({"node": id.0, "id": c.id}));
        n.motion.clips.push(c);
        n.motion.normalize();
    }
    doc.mark_dirty(ids);
    Ok(json!({"clips": clips}))
}

fn clip_update(s: &mut Session, p: &Value) -> Result<Value> {
    let node = req_id("motion.clipUpdate", p, "node")?;
    let id = req_num("motion.clipUpdate", p, "id")?.max(0.0) as u32;
    let mut clip = parse::<Clip>("motion.clipUpdate", p, "clip")?.ok_or_else(|| bad("motion.clipUpdate", "`clip` is required"))?;
    clip.id = id;
    clip.sanitize();
    let doc = s.doc_mut()?;
    let m = motion_mut(doc, node)?;
    let Some(c) = m.clips.iter_mut().find(|c| c.id == id) else {
        return Err(bad("motion.clipUpdate", format!("layer {} has no clip {id}", node.0)));
    };
    *c = clip;
    m.normalize();
    doc.mark_dirty([node]);
    Ok(json!({"ok": true}))
}

fn clip_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let node = req_id("motion.clipDelete", p, "node")?;
    let id = req_num("motion.clipDelete", p, "id")?.max(0.0) as u32;
    let doc = s.doc_mut()?;
    let m = motion_mut(doc, node)?;
    let before = m.clips.len();
    m.clips.retain(|c| c.id != id);
    let removed = m.clips.len() != before;
    m.normalize();
    doc.mark_dirty([node]);
    Ok(json!({"removed": removed}))
}

fn track_visible(s: &mut Session, p: &Value) -> Result<Value> {
    let node = req_id("motion.track", p, "node")?;
    let prop = prop_param("motion.track", p)?;
    let wanted = boolean(p, "visible");
    let doc = s.doc_mut()?;
    let m = motion_mut(doc, node)?;
    let name = prop.name();
    let Some(tr) = m.track_mut(prop) else {
        return Err(bad("motion.track", format!("layer {} has no {name} track", node.0)));
    };
    tr.visible = wanted.unwrap_or(!tr.visible);
    let visible = tr.visible;
    doc.mark_dirty([node]);
    Ok(json!({"visible": visible}))
}

fn motion_replace(s: &mut Session, p: &Value) -> Result<Value> {
    let node = req_id("motion.replace", p, "node")?;
    let mut m = parse::<NodeMotion>("motion.replace", p, "motion")?.ok_or_else(|| bad("motion.replace", "`motion` is required"))?;
    m.normalize();
    let doc = s.doc_mut()?;
    *motion_mut(doc, node)? = m;
    doc.mark_dirty([node]);
    Ok(json!({"ok": true}))
}

fn motion_timeline(s: &mut Session, p: &Value) -> Result<Value> {
    let page = match opt_id("motion.timeline", p, "page")? {
        Some(page) => page,
        None => s.page()?,
    };
    let mut settings = s.doc()?.timelines.get(&page).copied().unwrap_or_default();
    if let Some(d) = num(p, "duration") {
        settings.duration_ms = d.max(0.0) as u32;
    }
    if let Some(f) = num(p, "fps") {
        settings.fps = f.max(0.0) as u32;
    }
    if let Some(l) = p.get("loop").and_then(Value::as_bool) {
        settings.looping = l;
    }
    let settings = settings.sanitized();
    let doc = s.doc_mut()?;
    if settings.is_default() {
        doc.timelines.remove(&page);
    } else {
        doc.timelines.insert(page, settings);
    }
    doc.mark_dirty([page]);
    Ok(json!({"duration": settings.duration_ms, "fps": settings.fps, "loop": settings.looping}))
}

#[cfg(test)]
mod tests {
    use crate::Session;
    use serde_json::json;

    #[test]
    fn setting_the_defaults_takes_the_page_out_of_the_timelines() {
        let mut s = Session::new();
        let page = s.page().expect("a page");
        s.execute("motion.timeline", &json!({"duration": 5000, "fps": 30, "loop": true})).expect("set");
        assert!(s.doc().expect("doc").timelines.contains_key(&page));
        s.execute("motion.timeline", &json!({"duration": 2000, "fps": 60, "loop": false})).expect("reset");
        assert!(!s.doc().expect("doc").timelines.contains_key(&page), "a page at its defaults stays out of the file");
    }
}
