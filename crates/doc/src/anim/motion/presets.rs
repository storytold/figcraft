//! The animation preset library (Figma Motion's "Animations" popover).
//!
//! Every preset is a name, a group, a speed and a function that builds its [`Clip`] list. All 41
//! presets live in the one [`PRESETS`] table at the bottom of this file.

use super::clip::{Clip, ClipDirection, ClipKind, ClipMode, ClipRepeat};
use super::easing::Ease;
use figcraft_geom::Vec2;
use ClipDirection::*;
use ClipMode::{In, Out};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PresetGroup {
    Basic,
    Enter,
    Cycle,
    Exit,
}

impl PresetGroup {
    pub const ALL: [PresetGroup; 4] = [PresetGroup::Basic, PresetGroup::Enter, PresetGroup::Cycle, PresetGroup::Exit];

    pub fn name(&self) -> &'static str {
        match self {
            PresetGroup::Basic => "Basic",
            PresetGroup::Enter => "Enter",
            PresetGroup::Cycle => "Cycle",
            PresetGroup::Exit => "Exit",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PresetSpeed {
    Fast,
    Medium,
    Slow,
}

impl PresetSpeed {
    pub fn name(&self) -> &'static str {
        match self {
            PresetSpeed::Fast => "Fast",
            PresetSpeed::Medium => "Medium",
            PresetSpeed::Slow => "Slow",
        }
    }
}

/// One entry of the library.
pub struct Preset {
    pub name: &'static str,
    pub group: PresetGroup,
    /// `None` for the Basic group, which has no speed tag.
    pub speed: Option<PresetSpeed>,
    build: fn(u32) -> Vec<Clip>,
}

impl Preset {
    /// Length in ms of the preset's main motion (one cycle for Cycle presets).
    pub fn base_duration_ms(&self) -> u32 {
        match (self.group, self.speed) {
            (PresetGroup::Cycle, Some(PresetSpeed::Fast)) => 400,
            (PresetGroup::Cycle, Some(PresetSpeed::Medium)) => 1000,
            (PresetGroup::Cycle, Some(PresetSpeed::Slow)) => 2000,
            (_, Some(PresetSpeed::Fast)) => 300,
            (_, Some(PresetSpeed::Medium)) => 500,
            (_, Some(PresetSpeed::Slow)) => 800,
            (_, None) => 500,
        }
    }

    /// The preset's clips starting at time 0, with ids `0..n`.
    pub fn clips(&self) -> Vec<Clip> {
        self.instantiate(0, 0)
    }

    /// The preset's clips starting at `start_ms` (the playhead), with ids `first_id..`.
    pub fn instantiate(&self, start_ms: u32, first_id: u32) -> Vec<Clip> {
        let mut clips = (self.build)(self.base_duration_ms());
        for (i, c) in clips.iter_mut().enumerate() {
            c.id = first_id + i as u32;
            c.delay_ms = c.delay_ms.saturating_add(start_ms);
            c.sanitize();
        }
        clips
    }

    /// Whether the preset loops until the end of the timeline.
    pub fn loops(&self) -> bool {
        self.group == PresetGroup::Cycle
    }
}

/// Looks a preset up by name (case-insensitive).
pub fn find_preset(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name.eq_ignore_ascii_case(name))
}

/// Presets of one group in library order.
pub fn presets_in(group: PresetGroup) -> impl Iterator<Item = &'static Preset> {
    PRESETS.iter().filter(move |p| p.group == group)
}

/// Easing captured from Figma's Burst preset: `0.07, 0.97, 0.58, 1`.
const BURST: Ease = Ease::CustomBezier([0.07, 0.97, 0.58, 1.0]);
const OUT: Ease = Ease::EaseOut;
const IN: Ease = Ease::EaseIn;
const INOUT: Ease = Ease::EaseInOut;
const BACK: Ease = Ease::EaseOutBack;

fn mv(mode: ClipMode, dir: ClipDirection, dist: f32, d: u32, e: Ease) -> Clip {
    Clip::new(ClipKind::Position, mode).dir(dir).amount(dist).dur(d).ease(e)
}
fn grow(mode: ClipMode, factor: f32, d: u32, e: Ease) -> Clip {
    Clip::new(ClipKind::Scale, mode).amount(factor).dur(d).ease(e)
}
fn grow_axis(mode: ClipMode, axis: ClipDirection, factor: f32, d: u32, e: Ease) -> Clip {
    Clip::new(ClipKind::Scale, mode).dir(axis).amount(factor).dur(d).ease(e)
}
fn turn(mode: ClipMode, dir: ClipDirection, deg: f32, d: u32, e: Ease) -> Clip {
    Clip::new(ClipKind::Rotation, mode).dir(dir).amount(deg).dur(d).ease(e)
}
fn fade(mode: ClipMode, to: f32, d: u32, e: Ease) -> Clip {
    Clip::new(ClipKind::Opacity, mode).amount(to).dur(d).ease(e)
}
fn forever(c: Clip) -> Clip {
    c.repeat(ClipRepeat::Infinite)
}
/// Back-and-forth cycle: `d` is the full period, each leg lasts half of it.
fn wobble(c: Clip, d: u32) -> Clip {
    forever(c.dur(d / 2).yoyo())
}

macro_rules! preset {
    ($name:expr, $group:ident, $speed:expr, $build:expr) => {
        Preset { name: $name, group: PresetGroup::$group, speed: $speed, build: $build }
    };
}

const FAST: Option<PresetSpeed> = Some(PresetSpeed::Fast);
const MED: Option<PresetSpeed> = Some(PresetSpeed::Medium);
const SLOW: Option<PresetSpeed> = Some(PresetSpeed::Slow);

/// The whole library, in the order of Figma's popover: Basic (5), Enter (12), Cycle (12), Exit (12).
pub static PRESETS: &[Preset] = &[
    // ---- Basic: a single clip each ------------------------------------------------------------
    preset!("Position", Basic, None, |d| vec![mv(In, Bottom, 100.0, d, OUT)]),
    preset!("Scale", Basic, None, |d| vec![grow(In, 0.5, d, OUT)]),
    preset!("Rotation", Basic, None, |d| vec![turn(In, Clockwise, 90.0, d, OUT)]),
    preset!("Size", Basic, None, |d| vec![Clip::new(ClipKind::Size, In).amount(0.5).dur(d).ease(OUT)]),
    preset!("Opacity", Basic, None, |d| vec![fade(In, 0.0, d, OUT)]),
    // ---- Enter, fast (300ms) ------------------------------------------------------------------
    preset!("Burst", Enter, FAST, |d| vec![
        mv(In, Bottom, 116.0, d, BURST),
        grow(In, 0.8, d, BURST),
        turn(In, CounterClockwise, 10.0, d, BURST),
        fade(In, 0.0, d, BURST),
    ]),
    preset!("Ignite", Enter, FAST, |d| vec![grow(In, 0.4, d, BACK), fade(In, 0.0, d * 2 / 3, OUT)]),
    preset!("Sweep", Enter, FAST, |d| vec![mv(In, Left, 200.0, d, OUT), fade(In, 0.0, d, OUT)]),
    preset!("Slam", Enter, FAST, |d| vec![grow(In, 1.6, d, OUT), fade(In, 0.0, d / 2, Ease::Linear)]),
    // ---- Enter, medium (500ms) ----------------------------------------------------------------
    preset!("Descend", Enter, MED, |d| vec![mv(In, Top, 80.0, d, OUT), fade(In, 0.0, d, OUT)]),
    preset!("Magnify", Enter, MED, |d| vec![grow(In, 0.6, d, OUT), fade(In, 0.0, d, OUT)]),
    preset!("Glide", Enter, MED, |d| vec![mv(In, Right, 120.0, d, INOUT), fade(In, 0.0, d, INOUT)]),
    preset!("Drop", Enter, MED, |d| vec![mv(In, Top, 160.0, d, BACK), fade(In, 0.0, d / 3, OUT)]),
    // ---- Enter, slow (800ms) ------------------------------------------------------------------
    preset!("Reveal", Enter, SLOW, |d| vec![fade(In, 0.0, d, INOUT), grow(In, 0.94, d, INOUT)]),
    preset!("Rise", Enter, SLOW, |d| vec![mv(In, Bottom, 60.0, d, OUT), fade(In, 0.0, d, OUT)]),
    preset!("Tip", Enter, SLOW, |d| vec![turn(In, Clockwise, 25.0, d, BACK), fade(In, 0.0, d / 2, OUT)]),
    preset!("Feather", Enter, SLOW, |d| vec![mv(In, Bottom, 16.0, d, INOUT), fade(In, 0.0, d, INOUT)]),
    // ---- Cycle, fast (400ms period) -----------------------------------------------------------
    preset!("Crank", Cycle, FAST, |d| vec![forever(turn(Out, Clockwise, 360.0, d, INOUT))]),
    preset!("Press", Cycle, FAST, |d| vec![wobble(grow(Out, 0.9, d, INOUT), d)]),
    preset!("Pop", Cycle, FAST, |d| vec![wobble(grow(Out, 1.15, d, OUT), d)]),
    preset!("Scoot", Cycle, FAST, |d| vec![wobble(mv(Out, Right, 16.0, d, INOUT), d)]),
    // ---- Cycle, medium (1s period) ------------------------------------------------------------
    preset!("Spin", Cycle, MED, |d| vec![forever(turn(Out, Clockwise, 360.0, d, Ease::Linear))]),
    preset!("Flip", Cycle, MED, |d| vec![wobble(grow_axis(Out, Horizontal, -1.0, d, INOUT), d)]),
    preset!("Tick", Cycle, MED, |d| vec![wobble(turn(Out, Clockwise, 12.0, d, BACK), d)]),
    preset!("Creep", Cycle, MED, |d| vec![wobble(mv(Out, BottomRight, 20.0, d, INOUT), d)]),
    // ---- Cycle, slow (2s period) --------------------------------------------------------------
    preset!("Float", Cycle, SLOW, |d| vec![wobble(mv(Out, Top, 14.0, d, INOUT), d)]),
    preset!("Orbit", Cycle, SLOW, |d| {
        // A circle of radius 24 from two quarter-period shifted back-and-forth sweeps.
        let r = 24.0;
        let sweep = |from: Vec2, to: Vec2, delay: u32| wobble(Clip::new(ClipKind::Position, Out).custom(from, to).ease(INOUT).delay(delay), d);
        vec![sweep(Vec2::new(-r, 0.0), Vec2::new(r, 0.0), 0), sweep(Vec2::new(0.0, -r), Vec2::new(0.0, r), d / 4)]
    }),
    preset!("Breathe", Cycle, SLOW, |d| vec![wobble(grow(Out, 1.06, d, INOUT), d), wobble(fade(Out, 0.85, d, INOUT), d)]),
    preset!("Pulse", Cycle, SLOW, |d| vec![wobble(grow(Out, 1.1, d, OUT), d), wobble(fade(Out, 0.7, d, OUT), d)]),
    // ---- Exit, fast (300ms) -------------------------------------------------------------------
    preset!("Launch", Exit, FAST, |d| vec![mv(Out, Top, 240.0, d, IN), grow(Out, 0.8, d, IN), fade(Out, 0.0, d, IN)]),
    preset!("Vanish", Exit, FAST, |d| vec![grow(Out, 0.0, d, IN), fade(Out, 0.0, d, IN)]),
    preset!("Flick", Exit, FAST, |d| vec![mv(Out, Right, 160.0, d, IN), turn(Out, Clockwise, 25.0, d, IN), fade(Out, 0.0, d, IN)]),
    preset!("Swipe", Exit, FAST, |d| vec![mv(Out, Left, 200.0, d, IN), fade(Out, 0.0, d, IN)]),
    // ---- Exit, medium (500ms) -----------------------------------------------------------------
    preset!("Poof", Exit, MED, |d| vec![grow(Out, 1.4, d, OUT), fade(Out, 0.0, d, OUT)]),
    preset!("Deflate", Exit, MED, |d| vec![grow(Out, 0.5, d, IN), fade(Out, 0.0, d, IN)]),
    preset!("Fling", Exit, MED, |d| vec![mv(Out, TopRight, 200.0, d, IN), turn(Out, Clockwise, 45.0, d, IN), fade(Out, 0.0, d, IN)]),
    preset!("Splat", Exit, MED, |d| vec![grow_axis(Out, Horizontal, 1.4, d, OUT), grow_axis(Out, Vertical, 0.3, d, OUT), fade(Out, 0.0, d, IN),]),
    // ---- Exit, slow (800ms) -------------------------------------------------------------------
    preset!("Evaporate", Exit, SLOW, |d| vec![fade(Out, 0.0, d, INOUT), mv(Out, Top, 40.0, d, INOUT)]),
    preset!("Recede", Exit, SLOW, |d| vec![grow(Out, 0.7, d, INOUT), fade(Out, 0.0, d, INOUT)]),
    preset!("Sink", Exit, SLOW, |d| vec![mv(Out, Bottom, 80.0, d, INOUT), fade(Out, 0.0, d, INOUT)]),
    preset!("Tumble", Exit, SLOW, |d| vec![turn(Out, CounterClockwise, 120.0, d, IN), mv(Out, Bottom, 140.0, d, IN), fade(Out, 0.0, d, IN)]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_matches_the_figma_list() {
        let names = |g| presets_in(g).map(|p| p.name).collect::<Vec<_>>();
        assert_eq!(names(PresetGroup::Basic), ["Position", "Scale", "Rotation", "Size", "Opacity"]);
        assert_eq!(names(PresetGroup::Enter), ["Burst", "Ignite", "Sweep", "Slam", "Descend", "Magnify", "Glide", "Drop", "Reveal", "Rise", "Tip", "Feather"]);
        assert_eq!(names(PresetGroup::Cycle), ["Crank", "Press", "Pop", "Scoot", "Spin", "Flip", "Tick", "Creep", "Float", "Orbit", "Breathe", "Pulse"]);
        assert_eq!(
            names(PresetGroup::Exit),
            ["Launch", "Vanish", "Flick", "Swipe", "Poof", "Deflate", "Fling", "Splat", "Evaporate", "Recede", "Sink", "Tumble"]
        );
        assert_eq!(PRESETS.len(), 41);
        assert_eq!(find_preset("burst").unwrap().name, "Burst");
        assert!(find_preset("nope").is_none());
    }

    #[test]
    fn burst_matches_the_capture() {
        let clips = find_preset("Burst").unwrap().instantiate(997, 0);
        let kinds: Vec<ClipKind> = clips.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [ClipKind::Position, ClipKind::Scale, ClipKind::Rotation, ClipKind::Opacity]);
        assert!(clips.iter().all(|c| c.delay_ms == 997 && c.duration_ms == 300 && c.easing == BURST));
        assert_eq!(clips[0].amount, 116.0);
    }

    #[test]
    fn every_preset_has_valid_clips_with_unique_ids() {
        for p in PRESETS {
            let clips = p.instantiate(250, 7);
            assert!(!clips.is_empty(), "{}", p.name);
            let mut ids: Vec<u32> = clips.iter().map(|c| c.id).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), clips.len(), "{}", p.name);
            assert_eq!(ids[0], 7);
        }
    }

    #[test]
    fn enter_presets_start_changed_and_settle_at_rest() {
        for p in presets_in(PresetGroup::Enter) {
            let clips = p.clips();
            let moved = clips.iter().any(|c| {
                let v = c.value_at(0.0);
                let n = c.kind.neutral();
                (v.x - n.x).abs() > 1e-3 || (v.y - n.y).abs() > 1e-3
            });
            assert!(moved, "{} starts away from rest", p.name);
            for c in &clips {
                let v = c.value_at(10_000.0);
                let n = c.kind.neutral();
                assert!((v.x - n.x).abs() < 1e-3 && (v.y - n.y).abs() < 1e-3, "{} {:?} settles at rest", p.name, c.kind);
            }
        }
    }

    #[test]
    fn instantiate_shifts_and_offsets_ids() {
        let p = find_preset("Orbit").unwrap();
        let base = p.clips();
        let moved = p.instantiate(500, 3);
        assert_eq!(moved[1].delay_ms, base[1].delay_ms + 500);
        assert_eq!(moved[0].id, 3);
        assert_eq!(moved[1].id, 4);
    }
}
