//! Animation commands: expose the prototype transition geometry to the UI and the control channel.
//!
//! The playback itself lives in `figcraft_doc::anim`; these commands are thin entry points so the
//! canvas, the prototype panel and the agent control channel can drive transitions without knowing
//! the geometry.

use figcraft_doc::proto::Transition;
use figcraft_geom::Vec2;
use serde_json::{Value, json};
use crate::params::*;
use crate::{Result, Session, spec};
use super::CommandSpec;

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "animation.transitionPoses",
        "Transition poses",
        "transition: a Transition object (type + optional direction), progress: 0..1, width, height → the from/to pose (offset + alpha) of a prototype transition.",
        false,
        transition_poses
    )]
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
