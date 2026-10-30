//! Animation: prototype transition geometry, Smart Animate interpolation and a minimal playback
//! core, plus the timeline keyframe data model.
//!
//! The code stays free of any UI or rendering dependency so the engine, the canvas and the
//! timeline editor can all share it.

pub mod motion;
pub mod player;
pub mod smart;
pub mod transitions;

pub use motion::*;
pub use player::*;
pub use smart::*;
pub use transitions::*;
