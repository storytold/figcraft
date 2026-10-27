//! Animation commands: expose the prototype transition geometry to the UI and the control channel.
//!
//! The playback itself lives in `figcraft_doc::anim`; these commands are thin entry points so the
//! canvas, the prototype panel and the agent control channel can drive transitions without knowing
//! the geometry.

use figcraft_doc::anim::motion::{self, AnimProperty};
use figcraft_doc::proto::Transition;
use figcraft_geom::Vec2;
use serde_json::{Value, json};
use crate::params::*;
use crate::{Result, Session, spec};
use super::CommandSpec;

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "animation.transitionPoses",
            "Transition poses",
            "transition: a Transition object (type + optional direction), progress: 0..1, width, height → the from/to pose (offset + alpha) of a prototype transition.",
            false,
            transition_poses
        ),
        spec!(
            "motion.preset",
            "Animation preset",
            "preset: a library preset name (Burst, Fade…), at: start in ms (default 0) → applies it to the selected layers.",
            true,
            motion_preset
        ),
        spec!(
            "motion.keyframe",
            "Keyframe",
            "property: a property name (Opacity, Position, Width…), time: ms → adds a keyframe at the playhead to the selected layers.",
            true,
            motion_keyframe
        ),
        spec!(
            "motion.timeline",
            "Motion timeline",
            "duration: ms, fps, loop: bool → sets the current page's motion timeline.",
            true,
            motion_timeline
        ),
    ]
}

fn motion_preset(s: &mut Session, p: &Value) -> Result<Value> {
    let name = p.get("preset").and_then(Value::as_str).unwrap_or_default();
    let at = num(p, "at").unwrap_or(0.0).max(0.0) as u32;
    let ids = s.selection();
    let Some(preset) = motion::find_preset(name) else {
        return Ok(json!({"applied": 0}));
    };
    let doc = s.doc_mut()?;
    let mut applied = 0;
    for id in &ids {
        if let Some(node) = doc.node_mut(*id) {
            let first = node.motion.next_clip_id();
            node.motion.clips.extend(preset.instantiate(at, first));
            node.motion.normalize();
            applied += 1;
        }
    }
    doc.mark_dirty(ids);
    Ok(json!({"applied": applied}))
}

fn property_of(name: &str) -> Option<AnimProperty> {
    AnimProperty::ALL.iter().copied().find(|p| p.name().eq_ignore_ascii_case(name))
}

fn motion_keyframe(s: &mut Session, p: &Value) -> Result<Value> {
    let name = p.get("property").and_then(Value::as_str).unwrap_or("Opacity");
    let time = num(p, "time").unwrap_or(0.0).max(0.0) as u32;
    let Some(property) = property_of(name) else {
        return Ok(json!({"added": 0}));
    };
    let ids = s.selection();
    let doc = s.doc_mut()?;
    let mut added = 0;
    for id in &ids {
        let value = doc.node(*id).and_then(|n| motion::keyframed_value_at(n, property, time as f64));
        let Some(value) = value else { continue };
        if let Some(node) = doc.node_mut(*id) {
            node.motion.ensure_track(property).set_key(time, value, None);
            added += 1;
        }
    }
    doc.mark_dirty(ids);
    Ok(json!({"added": added}))
}

fn motion_timeline(s: &mut Session, p: &Value) -> Result<Value> {
    let page = s.page()?;
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
    s.doc_mut()?.timelines.insert(page, settings);
    Ok(json!({"duration": settings.duration_ms, "fps": settings.fps, "loop": settings.looping}))
}

fn transition_poses(_s: &mut Session, p: &Value) -> Result<Value> {
    let transition = parse::<Transition>("animation.transitionPoses", p, "transition")?.unwrap_or(Transition::Dissolve);
    let progress = num(p, "progress").unwrap_or(0.0);
    let w = num(p, "width").unwrap_or(0.0);
    let h = num(p, "height").unwrap_or(0.0);
    let poses = figcraft_doc::anim::transitions::poses(&transition, progress, Vec2::new(w, h));
    Ok(json!({
        "from": { "offset": [poses.from.offset.x, poses.from.offset.y], "alpha": poses.from.alpha },
        "to": { "offset": [poses.to.offset.x, poses.to.offset.y], "alpha": poses.to.alpha },
        "toOnTop": poses.to_on_top,
    }))
}
