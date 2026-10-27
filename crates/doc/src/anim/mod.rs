//! Animation: prototype transition geometry, Smart Animate interpolation and a minimal playback
//! core, plus the timeline keyframe data model.
//!
//! Ported from the desigma project's prototype/motion system and adapted to FigCraft's document
//! model (kurbo transforms, `width`/`height`, [`Corners`], [`StrokeProps`]). The code stays free of
//! any UI or rendering dependency so the engine, the canvas and a future timeline editor can all
//! share it.
//!
//! NOTE: this is a foundation. The timeline-based motion editor (desigma's `motion/` module) and
//! the full presentation player orchestration are NOT yet ported — see `motion/` and the PR
//! description.

pub mod motion;
pub mod player;
pub mod smart;
pub mod transitions;

pub use motion::*;
pub use player::*;
pub use smart::*;
pub use transitions::*;
