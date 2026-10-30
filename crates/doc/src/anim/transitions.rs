//! Pure geometry for prototype transitions and overlays.
//!
//! Works on [`Transition`]/[`Direction`]/[`OverlayPosition`] and has no document or UI
//! dependency — the player and the prototype panels call into this.

use crate::proto::{Direction, OverlayPosition, Transition};
use figcraft_geom::{Point, Rect, Vec2};

/// A connector ("noodle") from a hotspot to a destination frame: a cubic bezier leaving the
/// hotspot's right edge and entering the destination's left edge, plus an arrow head.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Noodle {
    pub p0: Point,
    pub c0: Point,
    pub c1: Point,
    pub p1: Point,
    /// Arrow head: tip first.
    pub arrow: [Point; 3],
}

pub const ARROW_LEN: f64 = 9.0;

/// Where a layer's hotspot sits: the middle of its right edge.
pub fn hotspot_center(src: Rect) -> Point {
    Point::new(src.x1, (src.y0 + src.y1) / 2.0)
}

/// Connector from `src` (screen rect of the interactive layer) to `dst` (screen rect of the
/// destination frame).
pub fn noodle(src: Rect, dst: Rect) -> Noodle {
    let p0 = hotspot_center(src);
    // Land where the line from the source meets the frame's left edge, kept clear of the corners.
    let margin = (dst.height() * 0.5).min(16.0);
    let y1 = p0.y.clamp(dst.y0 + margin, dst.y1 - margin);
    let p1 = Point::new(dst.x0, y1);
    let reach = ((p1.x - p0.x).abs() * 0.5).clamp(40.0, 260.0);
    let c0 = Point::new(p0.x + reach, p0.y);
    let c1 = Point::new(p1.x - reach, p1.y);
    let l = ARROW_LEN;
    Noodle { p0, c0, c1, p1, arrow: [p1, Point::new(p1.x - l, p1.y - l * 0.6), Point::new(p1.x - l, p1.y + l * 0.6)] }
}

impl Noodle {
    /// Point on the curve at `t` (0..1).
    pub fn point(&self, t: f64) -> Point {
        let u = 1.0 - t;
        let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
        let pts = [self.p0, self.c0, self.c1, self.p1];
        let mut out = Point::ZERO;
        for (wi, p) in w.iter().zip(pts) {
            out.x += wi * p.x;
            out.y += wi * p.y;
        }
        out
    }

    /// Where the stroke ends so it does not poke through the arrow head.
    pub fn stroke_end(&self) -> Point {
        Point::new(self.p1.x - ARROW_LEN * 0.8, self.p1.y)
    }

    /// The curve shortened to just before the arrow head.
    pub fn trimmed(&self) -> [Point; 4] {
        let e = self.stroke_end();
        [self.p0, self.c0, Point::new(e.x - (self.p1.x - self.c1.x), e.y), e]
    }

    /// Distance from `p` to the curve (sampled), for hit testing.
    #[cfg(test)]
    pub fn distance_to(&self, p: Point) -> f64 {
        let mut best = f64::MAX;
        let mut prev = self.p0;
        for i in 1..=24 {
            let q = self.point(i as f64 / 24.0);
            best = best.min(dist_to_segment(p, prev, q));
            prev = q;
        }
        best
    }
}

#[cfg(test)]
fn dist_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.x * ab.x + ab.y * ab.y;
    if len2 < 1e-9 {
        return (p - a).length();
    }
    let t = (((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / len2).clamp(0.0, 1.0);
    (p - Point::new(a.x + ab.x * t, a.y + ab.y * t)).length()
}

// ----------------------------------------------------------------- overlays

/// Top-left of an overlay of `size` inside `base` (the frame it opens over, both in the frame's
/// coordinate space starting at 0,0). `trigger` is the rect of the layer that fired the action,
/// used by `Manual`.
pub fn overlay_origin(pos: OverlayPosition, offset: Vec2, size: Vec2, base: Vec2, trigger: Rect) -> Vec2 {
    let (cx, cy) = ((base.x - size.x) / 2.0, (base.y - size.y) / 2.0);
    let (rx, ry) = (base.x - size.x, base.y - size.y);
    let (x, y) = match pos {
        OverlayPosition::Center => (cx, cy),
        OverlayPosition::TopLeft => (0.0, 0.0),
        OverlayPosition::TopCenter => (cx, 0.0),
        OverlayPosition::TopRight => (rx, 0.0),
        OverlayPosition::BottomLeft => (0.0, ry),
        OverlayPosition::BottomCenter => (cx, ry),
        OverlayPosition::BottomRight => (rx, ry),
        // Anchored to the trigger layer, but kept inside the frame when it fits.
        OverlayPosition::Manual => {
            let (x, y) = (trigger.x0 + offset.x, trigger.y0 + offset.y);
            (if rx >= 0.0 { x.clamp(0.0, rx) } else { x }, if ry >= 0.0 { y.clamp(0.0, ry) } else { y })
        }
    };
    Vec2::new(x, y)
}

// -------------------------------------------------------------- transitions

/// Unit vector of the motion a direction stands for (Left = moving left).
pub fn dir_vec(d: Direction) -> Vec2 {
    match d {
        Direction::Left => Vec2::new(-1.0, 0.0),
        Direction::Right => Vec2::new(1.0, 0.0),
        Direction::Top => Vec2::new(0.0, -1.0),
        Direction::Bottom => Vec2::new(0.0, 1.0),
    }
}

pub fn opposite(d: Direction) -> Direction {
    match d {
        Direction::Left => Direction::Right,
        Direction::Right => Direction::Left,
        Direction::Top => Direction::Bottom,
        Direction::Bottom => Direction::Top,
    }
}

/// The transition played when going back through one.
pub fn reverse(t: Transition) -> Transition {
    match t {
        Transition::Dissolve | Transition::SmartAnimate | Transition::Instant => t,
        Transition::MoveIn { direction } => Transition::MoveOut { direction: opposite(direction) },
        Transition::MoveOut { direction } => Transition::MoveIn { direction: opposite(direction) },
        Transition::Push { direction } => Transition::Push { direction: opposite(direction) },
        Transition::SlideIn { direction } => Transition::SlideOut { direction: opposite(direction) },
        Transition::SlideOut { direction } => Transition::SlideIn { direction: opposite(direction) },
    }
}

/// How one layer of a transition is drawn: offset in viewport px, opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub offset: Vec2,
    pub alpha: f32,
}

/// Poses of the outgoing and incoming frame at eased progress `p` (0..1, may overshoot) in a
/// viewport of `size`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Poses {
    pub from: Pose,
    pub to: Pose,
    /// Draw the incoming frame above the outgoing one.
    pub to_on_top: bool,
}

/// Portion of the frame the underlying screen shifts by in Slide transitions.
pub const SLIDE_PARALLAX: f64 = 0.3;

fn mul(v: Vec2, sx: f64, sy: f64) -> Vec2 {
    Vec2::new(v.x * sx, v.y * sy)
}

/// Poses of the outgoing and incoming frame at eased progress `p` (0..1) in a viewport of `size`.
pub fn poses(t: &Transition, p: f64, size: Vec2) -> Poses {
    let still = Pose { offset: Vec2::ZERO, alpha: 1.0 };
    let at = |v: Vec2| Pose { offset: v, alpha: 1.0 };
    // Offsets keep the overshoot a back or spring easing produces; only alpha is a real 0..1
    // quantity, so only alpha is clamped.
    let p = if p.is_finite() { p } else { 1.0 };
    let alpha = p.clamp(0.0, 1.0) as f32;
    match *t {
        // SmartAnimate is rendered from interpolated layers; this is its fallback.
        Transition::Dissolve | Transition::SmartAnimate | Transition::Instant => {
            Poses { from: still, to: Pose { offset: Vec2::ZERO, alpha }, to_on_top: true }
        }
        Transition::MoveIn { direction } => {
            let v = dir_vec(direction);
            Poses { from: still, to: at(mul(v, -(1.0 - p) * size.x, -(1.0 - p) * size.y)), to_on_top: true }
        }
        Transition::MoveOut { direction } => {
            let v = dir_vec(direction);
            Poses { from: at(mul(v, p * size.x, p * size.y)), to: still, to_on_top: false }
        }
        Transition::Push { direction } => {
            let v = dir_vec(direction);
            Poses { from: at(mul(v, p * size.x, p * size.y)), to: at(mul(v, -(1.0 - p) * size.x, -(1.0 - p) * size.y)), to_on_top: true }
        }
        Transition::SlideIn { direction } => {
            let v = dir_vec(direction);
            Poses {
                from: at(mul(v, p * size.x * SLIDE_PARALLAX, p * size.y * SLIDE_PARALLAX)),
                to: at(mul(v, -(1.0 - p) * size.x, -(1.0 - p) * size.y)),
                to_on_top: true,
            }
        }
        Transition::SlideOut { direction } => {
            let v = dir_vec(direction);
            Poses {
                from: at(mul(v, p * size.x, p * size.y)),
                to: at(mul(v, -(1.0 - p) * size.x * SLIDE_PARALLAX, -(1.0 - p) * size.y * SLIDE_PARALLAX)),
                to_on_top: false,
            }
        }
    }
}

/// Pose of an overlay at eased progress `p` where 0 is hidden and 1 is shown, for the transition
/// it opens with (closing plays it backwards).
pub fn overlay_pose(t: &Transition, p: f64, overlay: Vec2) -> Pose {
    // An overlay slides in from the side, so its offset keeps the overshoot; alpha does not.
    let p = if p.is_finite() { p } else { 1.0 };
    match *t {
        Transition::MoveIn { direction }
        | Transition::SlideIn { direction }
        | Transition::Push { direction }
        | Transition::MoveOut { direction }
        | Transition::SlideOut { direction } => {
            let v = dir_vec(direction);
            // Moves in from the side it came from: offset shrinks to zero.
            let off = mul(v, -(1.0 - p) * overlay.x, -(1.0 - p) * overlay.y);
            Pose { offset: off, alpha: 1.0 }
        }
        Transition::Dissolve | Transition::SmartAnimate | Transition::Instant => Pose { offset: Vec2::ZERO, alpha: p.clamp(0.0, 1.0) as f32 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_geom::Rect;

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect::new(x, y, x + w, y + h)
    }

    #[test]
    fn noodle_leaves_right_and_enters_left_edge() {
        let n = noodle(r(100.0, 200.0, 80.0, 40.0), r(400.0, 100.0, 300.0, 600.0));
        assert_eq!(n.p0, Point::new(180.0, 220.0));
        assert_eq!(n.p1, Point::new(400.0, 220.0), "keeps the source height when inside the frame's span");
        assert!(n.c0.x > n.p0.x && (n.c0.y - n.p0.y).abs() < 1e-9, "exits horizontally");
        assert!(n.c1.x < n.p1.x && (n.c1.y - n.p1.y).abs() < 1e-9, "enters horizontally");
        assert_eq!(n.arrow[0], n.p1);
        assert!(n.arrow[1].x < n.p1.x && n.arrow[2].x < n.p1.x);
        assert!(((n.arrow[1].y + n.arrow[2].y) / 2.0 - n.p1.y).abs() < 1e-9);
    }

    #[test]
    fn noodle_end_is_clamped_into_the_frame() {
        let n = noodle(r(0.0, 0.0, 10.0, 10.0), r(300.0, 500.0, 100.0, 200.0));
        assert_eq!(n.p1.y, 500.0 + 16.0);
        let n = noodle(r(0.0, 900.0, 10.0, 10.0), r(300.0, 500.0, 100.0, 200.0));
        assert_eq!(n.p1.y, 700.0 - 16.0);
    }

    #[test]
    fn noodle_curve_endpoints_and_midpoint() {
        let n = noodle(r(0.0, 0.0, 10.0, 20.0), r(300.0, 0.0, 100.0, 100.0));
        assert_eq!(n.point(0.0), n.p0);
        assert!((n.point(1.0) - n.p1).length() < 1e-9);
        let mid = n.point(0.5);
        assert!(mid.x > n.p0.x && mid.x < n.p1.x);
        assert!(n.distance_to(mid) < 0.5);
    }

    #[test]
    fn backward_noodle_still_starts_right_and_ends_left() {
        let n = noodle(r(500.0, 100.0, 50.0, 40.0), r(0.0, 0.0, 200.0, 300.0));
        assert!(n.c0.x > n.p0.x);
        assert!(n.c1.x < n.p1.x);
    }

    #[test]
    fn trimmed_curve_stops_before_arrow() {
        let n = noodle(r(0.0, 0.0, 10.0, 20.0), r(300.0, 0.0, 100.0, 100.0));
        let t = n.trimmed();
        assert_eq!(t[0], n.p0);
        assert!(t[3].x < n.p1.x);
    }

    #[test]
    fn overlay_positions_in_a_390x844_frame() {
        let base = Vec2::new(390.0, 844.0);
        let ov = Vec2::new(200.0, 100.0);
        let z = Rect::ZERO;
        let o = |p| overlay_origin(p, Vec2::ZERO, ov, base, z);
        assert_eq!(o(OverlayPosition::Center), Vec2::new(95.0, 372.0));
        assert_eq!(o(OverlayPosition::TopLeft), Vec2::new(0.0, 0.0));
        assert_eq!(o(OverlayPosition::TopCenter), Vec2::new(95.0, 0.0));
        assert_eq!(o(OverlayPosition::TopRight), Vec2::new(190.0, 0.0));
        assert_eq!(o(OverlayPosition::BottomLeft), Vec2::new(0.0, 744.0));
        assert_eq!(o(OverlayPosition::BottomCenter), Vec2::new(95.0, 744.0));
        assert_eq!(o(OverlayPosition::BottomRight), Vec2::new(190.0, 744.0));
    }

    #[test]
    fn manual_overlay_follows_trigger_and_stays_inside() {
        let base = Vec2::new(400.0, 800.0);
        let ov = Vec2::new(100.0, 100.0);
        let trig = Rect::new(50.0, 60.0, 80.0, 90.0);
        assert_eq!(overlay_origin(OverlayPosition::Manual, Vec2::new(10.0, 40.0), ov, base, trig), Vec2::new(60.0, 100.0));
        let far = Rect::new(380.0, 790.0, 390.0, 800.0);
        assert_eq!(overlay_origin(OverlayPosition::Manual, Vec2::ZERO, ov, base, far), Vec2::new(300.0, 700.0));
        assert_eq!(overlay_origin(OverlayPosition::Manual, Vec2::new(-500.0, -500.0), ov, base, trig), Vec2::new(0.0, 0.0));
    }

    #[test]
    fn reverse_is_an_involution_and_flips_direction() {
        use Direction::*;
        for k in [
            Transition::Dissolve,
            Transition::SmartAnimate,
            Transition::MoveIn { direction: Left },
            Transition::MoveOut { direction: Top },
            Transition::Push { direction: Right },
            Transition::SlideIn { direction: Bottom },
            Transition::SlideOut { direction: Left },
        ] {
            assert_eq!(reverse(reverse(k)), k);
        }
        assert_eq!(reverse(Transition::MoveIn { direction: Direction::Left }), Transition::MoveOut { direction: Direction::Right });
        assert_eq!(reverse(Transition::Push { direction: Direction::Left }), Transition::Push { direction: Direction::Right });
    }

    #[test]
    fn poses_move_in_from_the_right_when_moving_left() {
        let size = Vec2::new(400.0, 800.0);
        let p0 = poses(&Transition::MoveIn { direction: Direction::Left }, 0.0, size);
        assert_eq!(p0.to.offset, Vec2::new(400.0, 0.0), "incoming starts one frame to the right");
        assert_eq!(p0.from.offset, Vec2::ZERO);
        assert!(p0.to_on_top);
        let p1 = poses(&Transition::MoveIn { direction: Direction::Left }, 1.0, size);
        assert_eq!(p1.to.offset, Vec2::ZERO);
        let half = poses(&Transition::MoveIn { direction: Direction::Left }, 0.5, size);
        assert_eq!(half.to.offset.x, 200.0);
    }

    #[test]
    fn poses_move_out_push_and_slide() {
        let size = Vec2::new(400.0, 800.0);
        let out = poses(&Transition::MoveOut { direction: Direction::Bottom }, 0.5, size);
        assert_eq!(out.from.offset, Vec2::new(0.0, 400.0));
        assert!(!out.to_on_top);
        let push = poses(&Transition::Push { direction: Direction::Left }, 0.25, size);
        assert_eq!(push.from.offset.x, -100.0);
        assert_eq!(push.to.offset.x, 300.0);
        let slide = poses(&Transition::SlideIn { direction: Direction::Left }, 1.0, size);
        assert!((slide.from.offset.x + 400.0 * SLIDE_PARALLAX).abs() < 1e-3);
        assert_eq!(slide.to.offset, Vec2::ZERO);
        let d = poses(&Transition::Dissolve, 0.3, size);
        assert!((d.to.alpha - 0.3).abs() < 1e-6 && d.from.alpha == 1.0);
        assert_eq!(poses(&Transition::Dissolve, 1.2, size).to.alpha, 1.0);
    }

    #[test]
    fn overlay_pose_slides_in_from_its_side_and_dissolves() {
        let ov = Vec2::new(300.0, 200.0);
        let hidden = overlay_pose(&Transition::MoveIn { direction: Direction::Top }, 0.0, ov);
        assert_eq!(hidden.offset, Vec2::new(0.0, 200.0), "moving up means it arrives from below");
        assert_eq!(overlay_pose(&Transition::MoveIn { direction: Direction::Top }, 1.0, ov).offset, Vec2::ZERO);
        assert_eq!(overlay_pose(&Transition::Dissolve, 0.5, ov).alpha, 0.5);
    }
}
