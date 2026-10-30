//! Animation: the timeline keyframe data model.
//!
//! The code stays free of any UI or rendering dependency so the engine, the canvas and the
//! timeline editor can all share it. Prototype transitions and Smart Animate play from
//! `present`, which already owns the poses and the interpolation.

pub mod motion;
