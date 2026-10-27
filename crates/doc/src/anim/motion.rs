//! Timeline-based motion design: keyframe tracks over a clip.
//!
//! This is the data model only — the keyframe/track/clip structures and a sampler. The timeline
//! editor UI (the track list, the playhead, the inspector) is NOT yet ported; see the PR
//! description. Keeping the model free of any UI dependency lets a future editor and an eventual
//! headless renderer share it.

use std::collections::BTreeMap;
use crate::proto::Easing;

/// One keyframe: a value at a time (seconds) with the easing into the next keyframe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe {
    pub time: f64,
    pub value: f64,
    pub easing: Easing,
}

impl Keyframe {
    pub fn new(time: f64, value: f64) -> Self {
        Self { time: if time.is_finite() { time.max(0.0) } else { 0.0 }, value, easing: Easing::EaseInOut }
    }
}

/// A property track: keyframes sorted by time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub keyframes: Vec<Keyframe>,
}

impl Track {
    pub fn new(keyframes: Vec<Keyframe>) -> Self {
        let mut k = keyframes;
        k.sort_by(|a, b| a.time.total_cmp(&b.time));
        Self { keyframes: k }
    }

    /// Sample the track's value at `t` (seconds), easing between the surrounding keyframes.
    pub fn sample(&self, t: f64) -> f64 {
        let ks = &self.keyframes;
        if ks.is_empty() {
            return 0.0;
        }
        if ks.len() == 1 || t <= ks[0].time {
            return ks[0].value;
        }
        if t >= ks[ks.len() - 1].time {
            return ks[ks.len() - 1].value;
        }
        for i in 0..ks.len() - 1 {
            let (a, b) = (&ks[i], &ks[i + 1]);
            if t >= a.time && t <= b.time {
                let span = (b.time - a.time).max(1e-9);
                let local = ((t - a.time) / span).clamp(0.0, 1.0);
                let eased = a.easing.eval(local);
                return a.value + (b.value - a.value) * eased;
            }
        }
        ks[ks.len() - 1].value
    }
}

/// A motion clip: named tracks sharing a duration.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MotionClip {
    pub duration: f64,
    pub tracks: BTreeMap<String, Track>,
}

impl MotionClip {
    /// Sample every track at `t` (seconds).
    pub fn sample(&self, t: f64) -> BTreeMap<String, f64> {
        self.tracks.iter().map(|(name, tr)| (name.clone(), tr.sample(t))).collect()
    }

    /// Sample one named track, if it exists.
    pub fn sample_track(&self, name: &str, t: f64) -> Option<f64> {
        self.tracks.get(name).map(|tr| tr.sample(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::Easing;

    #[test]
    fn track_samples_endpoints_and_eases() {
        let tr = Track::new(vec![
            Keyframe::new(0.0, 0.0),
            Keyframe { time: 1.0, value: 100.0, easing: Easing::Linear },
        ]);
        assert_eq!(tr.sample(-1.0), 0.0);
        assert_eq!(tr.sample(1.0), 100.0);
        assert!((tr.sample(0.5) - 50.0).abs() < 1e-9, "linear midpoint");
    }

    #[test]
    fn track_without_keyframes_is_zero() {
        assert_eq!(Track::default().sample(0.5), 0.0);
    }

    #[test]
    fn clip_samples_all_tracks() {
        let mut clip = MotionClip { duration: 1.0, tracks: BTreeMap::new() };
        clip.tracks.insert(
            "x".into(),
            Track::new(vec![Keyframe::new(0.0, 0.0), Keyframe { time: 1.0, value: 10.0, easing: Easing::Linear }]),
        );
        clip.tracks.insert(
            "opacity".into(),
            Track::new(vec![Keyframe::new(0.0, 1.0), Keyframe { time: 1.0, value: 0.0, easing: Easing::Linear }]),
        );
        let s = clip.sample(0.5);
        assert_eq!(s["x"], 5.0);
        assert_eq!(s["opacity"], 0.5);
    }
}
