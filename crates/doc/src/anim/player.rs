//! Minimal playback core for prototype transitions.
//!
//! Ported from desigma's `proto/player.rs` (a much larger presentation runtime) and reduced to the
//! part FigCraft needs now: turn a [`Transition`] + [`Easing`] + duration into a time→visual
//! mapping — poses for the simple transitions, a morphed document for Smart Animate. The UI supplies
//! the clock and composites the two frames; this crate stays free of any rendering or UI dependency.

use figcraft_geom::Vec2;
use crate::anim::smart::SmartPlan;
use crate::anim::transitions::{poses, Poses};
use crate::proto::{Easing, Transition};
use crate::{Document, NodeId};

/// Drives one transition between two frames.
pub struct TransitionPlayer {
    pub transition: Transition,
    pub easing: Easing,
    pub duration_ms: u32,
}

impl TransitionPlayer {
    pub fn new(transition: Transition, easing: Easing, duration_ms: u32) -> Self {
        Self { transition, easing, duration_ms }
    }

    /// Seconds the transition plays for (a spring runs until it settles).
    pub fn total_seconds(&self) -> f64 {
        self.easing.seconds(self.duration_ms)
    }

    /// Eased progress (0..1) at `secs` since the start.
    pub fn progress(&self, secs: f64) -> f64 {
        let total = self.total_seconds();
        let raw = if total > 0.0 { (secs / total).clamp(0.0, 1.0) } else { 1.0 };
        self.easing.eval(raw)
    }

    pub fn is_done(&self, secs: f64) -> bool {
        secs >= self.total_seconds()
    }

    /// Visual poses of the outgoing and incoming frame at `secs` (viewport `size`).
    pub fn poses(&self, secs: f64, size: Vec2) -> Poses {
        poses(&self.transition, self.progress(secs), size)
    }
}

/// Drives a Smart Animate transition, producing a morphed document at each time.
pub struct SmartPlayer {
    plan: SmartPlan,
    easing: Easing,
    duration_ms: u32,
}

impl SmartPlayer {
    /// None when the two frames share no layer to morph (fall back to a dissolve).
    pub fn new(from_doc: &Document, from_root: NodeId, to_doc: &Document, to_root: NodeId, easing: Easing, duration_ms: u32) -> Option<Self> {
        SmartPlan::build(from_doc, from_root, to_doc, to_root).map(|plan| Self { plan, easing, duration_ms })
    }

    pub fn total_seconds(&self) -> f64 {
        self.easing.seconds(self.duration_ms)
    }

    pub fn progress(&self, secs: f64) -> f64 {
        let total = self.total_seconds();
        let raw = if total > 0.0 { (secs / total).clamp(0.0, 1.0) } else { 1.0 };
        self.easing.eval(raw)
    }

    pub fn is_done(&self, secs: f64) -> bool {
        secs >= self.total_seconds()
    }

    /// The morphed document at `secs` (0 = source look, 1 = destination look).
    pub fn frame(&mut self, secs: f64) -> (&Document, NodeId) {
        self.plan.apply(self.progress(secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::smart::SmartPlan;
    use crate::proto::Direction;
    use crate::NodeKind;

    fn frames() -> (Document, NodeId, NodeId) {
        let mut d = Document::new("t");
        let page = d.first_page().unwrap();
        let a = d.create(page, NodeKind::Frame, "A", 0.0, 0.0, 400.0, 800.0).unwrap();
        let b = d.create(page, NodeKind::Frame, "B", 0.0, 0.0, 400.0, 800.0).unwrap();
        (d, a, b)
    }

    #[test]
    fn transition_player_progress_clamps_and_eases() {
        let p = TransitionPlayer::new(Transition::MoveIn { direction: Direction::Left }, Easing::EaseOut, 300);
        assert!((p.progress(0.0) - 0.0).abs() < 1e-9);
        assert!((p.progress(10.0) - 1.0).abs() < 1e-9, "clamps to 1");
        let mid = p.poses(0.15, Vec2::new(400.0, 800.0));
        assert!(mid.to.offset.x > 0.0 && mid.to_on_top);
    }

    #[test]
    fn smart_player_morphs_and_resolves() {
        let (mut d, a, b) = frames();
        let _ = d.create(a, NodeKind::Rectangle, "Card", 0.0, 0.0, 100.0, 40.0).unwrap();
        let _ = d.create(b, NodeKind::Rectangle, "Card", 100.0, 60.0, 200.0, 80.0).unwrap();
        let mut player = SmartPlayer::new(&d, a, &d, b, Easing::Linear, 300).unwrap();
        // 300ms total, so 0.15s is the halfway point and 0.3s is the end.
        let (doc, root) = player.frame(0.15);
        let card = doc.node(doc.node(root).unwrap().children[0]).unwrap();
        assert!((card.width - 150.0).abs() < 1e-6, "halfway between 100 and 200");
        let (doc, root) = player.frame(0.3);
        let card = doc.node(doc.node(root).unwrap().children[0]).unwrap();
        assert!((card.width - 200.0).abs() < 1e-6, "reaches the destination width");
        assert!(player.is_done(0.3));
        // building a plan from scratch works too
        let _plan = SmartPlan::build(&d, a, &d, b);
    }
}
