//! Motion: keyframe animation.
//!
//! The animation model, evaluation and edit helpers, with no UI:
//!
//! * [`TimelineSettings`]: duration, fps and loop, stored **per page**.
//! * [`NodeMotion`]: keyframe [`Track`]s, one per [`AnimProperty`], plus [`Clip`]s.
//! * [`Ease`]: hold, linear, bezier and spring easing for the segment that starts at a keyframe.
//!
//! Times are milliseconds. Keyframes sit on whole milliseconds; the playhead may be fractional.

mod clip;
mod easing;
pub mod eval;
mod track;
mod value;

pub use clip::{Clip, ClipDirection, ClipKind, ClipMode, ClipRepeat};
pub use easing::{
    DEFAULT_ELASTICITY, ELASTICITY_RANGE, Ease, EaseFamily, SPRING_SETTLE_EPS, SPRING_SETTLE_K, SpringAlgorithm, cubic_bezier,
    spring_angular_frequency, spring_at, spring_damping_ratio, spring_overshoot, spring_settle_secs, spring_step, spring_stiffness_damping,
    spring_unit_response, unit_mass,
};
pub use eval::{
    EvaluatedNode, MotionFrame, MotionPlayer, animated_nodes, base_position, base_rotation_deg, base_value, evaluate, evaluate_frame, evaluate_node,
    first_drop_shadow, first_layer_blur, keyframed_value_at, value_at,
};
pub use track::{Keyframe, NodeMotion, Track};
pub use value::{AnimProperty, AnimValue, ValueKind, lerp_color, lerp_f64, lerp_vec2};

use serde::{Deserialize, Serialize};

fn default_duration() -> u32 {
    2000
}
fn default_fps() -> u32 {
    60
}

/// Timeline settings of one page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct TimelineSettings {
    /// Length of the timeline in ms. Default 2000.
    pub duration_ms: u32,
    /// Frames per second used for stepping, snapping and export. Default 60.
    pub fps: u32,
    /// Whether playback loops (the Loop button).
    #[serde(rename = "loop")]
    pub looping: bool,
}

/// The stored shape of [`TimelineSettings`], before the bounds are applied.
#[derive(Deserialize)]
struct RawTimeline {
    #[serde(default = "default_duration")]
    duration_ms: u32,
    #[serde(default = "default_fps")]
    fps: u32,
    #[serde(default, rename = "loop")]
    looping: bool,
}

/// Reading a timeline clamps it to the bounds a `motion.timeline` command writes: a hand-edited
/// file or a collaborator's entry carrying an absurd duration would otherwise reach the ruler,
/// whose tick loop would then never end and hang the UI thread.
impl<'de> Deserialize<'de> for TimelineSettings {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = RawTimeline::deserialize(d)?;
        Ok(Self { duration_ms: raw.duration_ms, fps: raw.fps, looping: raw.looping }.sanitized())
    }
}

impl Default for TimelineSettings {
    fn default() -> Self {
        Self { duration_ms: default_duration(), fps: default_fps(), looping: false }
    }
}

impl TimelineSettings {
    pub const MAX_DURATION_MS: u32 = 10 * 60 * 1000;
    pub const MAX_FPS: u32 = 240;

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Clamps to a usable range: duration 1ms ..= 10min, fps 1 ..= 240.
    pub fn sanitized(mut self) -> Self {
        self.duration_ms = self.duration_ms.clamp(1, Self::MAX_DURATION_MS);
        self.fps = self.fps.clamp(1, Self::MAX_FPS);
        self
    }

    /// Length of one frame in ms.
    pub fn frame_ms(&self) -> f32 {
        1000.0 / self.fps.max(1) as f32
    }

    /// Number of frames of a render of the whole timeline (at least 1).
    pub fn frame_count(&self) -> u32 {
        ((self.duration_ms as f64 * self.fps.max(1) as f64 / 1000.0).ceil() as u32).max(1)
    }

    /// Timeline time of frame `index`.
    pub fn frame_time_ms(&self, index: u32) -> f32 {
        index as f32 * self.frame_ms()
    }

    /// Nearest frame boundary to `t_ms`.
    pub fn snap_to_frame(&self, t_ms: f32) -> f32 {
        let f = self.frame_ms();
        (t_ms / f).round() * f
    }

    /// Maps a free-running playback clock to timeline time.
    pub fn playback_time(&self, elapsed_ms: f32) -> f32 {
        let d = self.duration_ms.max(1) as f32;
        if self.looping { elapsed_ms.rem_euclid(d) } else { elapsed_ms.clamp(0.0, d) }
    }

    /// Whether non-looping playback has reached the end at `elapsed_ms`.
    pub fn finished(&self, elapsed_ms: f32) -> bool {
        !self.looping && elapsed_ms >= self.duration_ms as f32
    }
}

#[cfg(test)]
mod tests {
    use super::{AnimProperty, AnimValue, Clip, ClipKind, ClipMode, Ease, TimelineSettings};

    #[test]
    fn defaults_are_two_seconds_at_60fps() {
        let s = TimelineSettings::default();
        assert_eq!((s.duration_ms, s.fps, s.looping), (2000, 60, false));
        assert!(s.is_default());
        assert_eq!(s.frame_count(), 120);
        assert!((s.frame_ms() - 16.6667).abs() < 1e-3);
    }

    #[test]
    fn serde_defaults_and_loop_name() {
        let s: TimelineSettings = serde_json::from_str("{}").unwrap();
        assert!(s.is_default());
        let s: TimelineSettings = serde_json::from_str(r#"{"loop":true,"duration_ms":500}"#).unwrap();
        assert!(s.looping && s.duration_ms == 500 && s.fps == 60);
        assert!(serde_json::to_string(&s).unwrap().contains("\"loop\":true"));
    }

    #[test]
    fn sanitize_and_playback_time() {
        let s = TimelineSettings { duration_ms: 0, fps: 0, looping: false }.sanitized();
        assert_eq!((s.duration_ms, s.fps), (1, 1));
        let s = TimelineSettings { duration_ms: 1000, fps: 30, looping: true };
        assert!((s.playback_time(2500.0) - 500.0).abs() < 1e-3);
        assert!(!s.finished(5000.0));
    }

    #[test]
    fn a_stored_timeline_reads_back_within_its_bounds() {
        // A hand-edited file, or a collaborator's entry, can carry any u32: the ruler's tick loop
        // runs to the duration, so reading clamps the same way `motion.timeline` writes.
        let s: TimelineSettings = serde_json::from_str(r#"{"duration_ms":4294967295,"fps":4294967295}"#).unwrap();
        assert_eq!((s.duration_ms, s.fps), (TimelineSettings::MAX_DURATION_MS, TimelineSettings::MAX_FPS));
        assert!(s.frame_count() <= TimelineSettings::MAX_DURATION_MS / 1000 * TimelineSettings::MAX_FPS + 1, "a finite frame count");
    }

    use crate::{Document, NodeKind};

    fn animated() -> (Document, crate::NodeId, crate::NodeId) {
        let mut d = Document::new("anim");
        let page = d.first_page().unwrap();
        let id = d.create(page, NodeKind::Rectangle, "Card", 10.0, 20.0, 100.0, 50.0).unwrap();
        let n = d.node_mut(id).unwrap();
        let pos = n.motion.ensure_track(AnimProperty::Position);
        pos.set_key(0, AnimValue::Vec2(figcraft_geom::Vec2::new(10.0, 20.0)), Some(Ease::bezier(0.07, 0.97, 0.58, 1.0)));
        pos.set_key(1500, AnimValue::Vec2(figcraft_geom::Vec2::new(300.0, 20.0)), None);
        n.motion.clips = vec![Clip::new(ClipKind::Opacity, ClipMode::In)];
        d.timelines.insert(page, TimelineSettings { duration_ms: 4000, fps: 30, looping: true });
        (d, page, id)
    }

    #[test]
    fn document_roundtrip_keeps_animation_data() {
        let (d, page, id) = animated();
        let json = serde_json::to_string(&d).unwrap();
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(back.node(id).unwrap().motion, d.node(id).unwrap().motion);
        assert_eq!(back.timelines.get(&page), d.timelines.get(&page));
        assert_eq!(serde_json::to_string(&back).unwrap(), json, "stable on a second pass");
    }

    #[test]
    fn a_deleted_page_takes_its_timeline_with_it() {
        let (mut d, page, _) = animated();
        let other = d.add_page("Other");
        assert!(d.timelines.contains_key(&page));
        d.remove(page).unwrap();
        assert!(!d.timelines.contains_key(&page), "the settings went with the page");
        assert!(d.pages.contains(&other));
    }

    #[test]
    fn plain_documents_serialize_without_animation_fields() {
        let mut d = Document::new("plain");
        let page = d.first_page().unwrap();
        d.create(page, NodeKind::Rectangle, "R", 0.0, 0.0, 5.0, 5.0).unwrap();
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("\"motion\"") && !json.contains("\"timelines\""), "old files stay byte-identical in shape");
        let back: Document = serde_json::from_str(&json).unwrap();
        assert!(back.timelines.is_empty());
    }
}
