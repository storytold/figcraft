//! Arc handles: the on-canvas handles of a selected ellipse's arc (as public documentation of
//! Figma-class tools describes them).
//!
//! - **Sweep** starts on the ellipse's right edge, on its centre line. Dragging it around the
//!   ellipse opens it into an arc: the sweep is a signed share of a full turn, positive dragging up
//!   from the right edge and negative dragging down. Brought back to the start, it closes the
//!   ellipse again.
//! - **Start** (drawn with a dot inside) shows once there is an arc. It turns the arc around the
//!   ellipse and keeps its sweep.
//! - **Ratio** sits at the centre (on a ring, at the inner edge, halfway round the arc). Dragging
//!   it outwards makes a ring. Dragged into the gap, it shows the other part of the ellipse.
//!
//! Shift snaps angles to 15° steps and the ratio to 5% steps. Positions are in page coordinates
//! and work for rotated and flipped ellipses at any zoom. The handles hide when the ellipse is too
//! small on screen. The Move tool's gesture (`tools.rs`) writes the result through `node.set`'s
//! `arcStart` / `arcSweep` / `arcInner` keys, one undo step per drag.

use std::f64::consts::TAU;

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_geom::{Affine, Point, Vec2};
use serde::Serialize;

/// One of an ellipse's arc handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ArcHandle {
    Sweep,
    Start,
    Ratio,
}

/// An ellipse's arc: start and sweep angles in radians (clockwise from the right, y down; a
/// negative sweep runs counter-clockwise) and the inner radius as a share of the outer one.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Arc {
    pub start: f64,
    pub sweep: f64,
    pub inner: f64,
}

impl Arc {
    /// A full turn (an ellipse or a ring, no gap).
    pub fn full(&self) -> bool {
        self.sweep.abs() >= TAU - 1e-9
    }

    /// Whether `angle` falls on the arc (not in its gap).
    fn covers(&self, angle: f64) -> bool {
        let along = if self.sweep >= 0.0 { angle - self.start } else { self.start - angle };
        self.full() || along.rem_euclid(TAU) <= self.sweep.abs()
    }

    /// The live readout while dragging `h`: `Sweep 75%`, `Start 90°` or `Ratio 40%`.
    pub fn label(&self, h: ArcHandle) -> String {
        match h {
            ArcHandle::Sweep => format!("Sweep {}%", one_decimal(self.sweep / TAU * 100.0)),
            ArcHandle::Start => format!("Start {}°", one_decimal(self.start.to_degrees().rem_euclid(360.0))),
            ArcHandle::Ratio => format!("Ratio {}%", one_decimal(self.inner * 100.0)),
        }
    }
}

/// `v` rounded to one decimal, without a trailing `.0` (or a minus on zero).
fn one_decimal(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    let r = if r == 0.0 { 0.0 } else { r };
    if r.fract() == 0.0 { format!("{r:.0}") } else { format!("{r:.1}") }
}

/// The selected ellipse's handles as drawn at a zoom (page coordinates). Start shows once there
/// is an arc, Ratio once there is an arc or a ring.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ArcHandles {
    pub id: NodeId,
    pub sweep: [f64; 2],
    pub start: Option<[f64; 2]>,
    pub ratio: Option<[f64; 2]>,
}

impl ArcHandles {
    /// Where handle `h` is, if shown.
    pub fn at(&self, h: ArcHandle) -> Option<[f64; 2]> {
        match h {
            ArcHandle::Sweep => Some(self.sweep),
            ArcHandle::Start => self.start,
            ArcHandle::Ratio => self.ratio,
        }
    }
}

/// The live readout of an arc handle drag: the handle, its label and where the handle is now.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ArcHint {
    pub handle: ArcHandle,
    pub label: String,
    pub at: [f64; 2],
}

/// Ellipses smaller than this on screen (either side, in pixels) show no arc handles.
pub const MIN_PX: f64 = 40.0;
/// How far inside the ellipse's edge the Sweep and Start handles sit (screen pixels), clear of
/// the resize edge.
const INSET_PX: f64 = 10.0;
/// How near the pointer must be to pick a handle (screen pixels).
const HIT_PX: f64 = 7.0;
/// The Sweep handle closes the ellipse when it comes this near the start (screen pixels).
const CLOSE_PX: f64 = 6.0;
/// Shift steps.
const SNAP_DEG: f64 = 15.0;
const SNAP_RATIO: f64 = 0.05;
/// The largest ratio `node.set`'s `arcInner` keeps.
const MAX_RATIO: f64 = 0.99;

/// An ellipse as its handles see it: its local → page transform, radii and arc.
struct Ellipse {
    t: Affine,
    rx: f64,
    ry: f64,
    arc: Arc,
}

impl Ellipse {
    fn of(doc: &Document, id: NodeId) -> Option<Self> {
        let n = doc.node(id)?;
        let NodeKind::Ellipse { start, sweep, inner } = n.kind else { return None };
        let (rx, ry) = (n.width / 2.0, n.height / 2.0);
        let t = doc.world_transform(id);
        let ok = rx.is_finite() && ry.is_finite() && rx > 0.0 && ry > 0.0 && t.determinant().is_finite() && t.determinant().abs() > 1e-12;
        let arc = Arc { start: finite(start), sweep: finite(sweep).clamp(-TAU, TAU), inner: finite(inner).clamp(0.0, 1.0) };
        ok.then_some(Self { t, rx, ry, arc })
    }

    /// The page point at angle `a`, `k` of the way from the centre to the edge.
    fn point(&self, a: f64, k: f64) -> Point {
        self.t * Point::new(self.rx * (1.0 + k * a.cos()), self.ry * (1.0 + k * a.sin()))
    }

    /// The edge point at angle `a`, pulled `INSET_PX` towards the centre (at most halfway).
    fn inset(&self, a: f64, zoom: f64) -> Point {
        let (e, c) = (self.point(a, 1.0), self.point(0.0, 0.0));
        let d = c - e;
        let len = d.hypot();
        if len <= 1e-12 {
            return e;
        }
        e + d * ((INSET_PX / zoom.max(1e-6)).min(len / 2.0) / len)
    }

    /// Where handle `h` is for `arc` at `zoom`.
    fn handle(&self, h: ArcHandle, arc: Arc, zoom: f64) -> Point {
        match h {
            ArcHandle::Sweep => self.inset(arc.start + arc.sweep, zoom),
            ArcHandle::Start => self.inset(arc.start, zoom),
            ArcHandle::Ratio => self.point(arc.start + arc.sweep / 2.0, arc.inner),
        }
    }

    /// Page point `p` on the unit circle the ellipse maps from: its angle and ratio.
    fn unit(&self, p: Point) -> Vec2 {
        let q = self.t.inverse() * p;
        Vec2::new(q.x / self.rx - 1.0, q.y / self.ry - 1.0)
    }

    /// Its smaller side on screen, in pixels.
    fn screen_px(&self, zoom: f64) -> f64 {
        let o = self.t * Point::ZERO;
        let w = (self.t * Point::new(2.0 * self.rx, 0.0) - o).hypot();
        let h = (self.t * Point::new(0.0, 2.0 * self.ry) - o).hypot();
        w.min(h) * zoom
    }
}

fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// `zoom` if usable, else 1.
fn sane_zoom(zoom: f64) -> f64 {
    if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 }
}

fn pt(p: Point) -> [f64; 2] {
    [p.x, p.y]
}

/// The ellipse whose arc handles show: the only selected layer, an unlocked visible ellipse at
/// least [`MIN_PX`] on screen.
fn target(doc: &Document, sel: &[NodeId], zoom: f64) -> Option<(NodeId, Ellipse)> {
    let [id] = sel else { return None };
    let n = doc.node(*id)?;
    if n.locked || !n.visible || !zoom.is_finite() || zoom <= 0.0 {
        return None;
    }
    let e = Ellipse::of(doc, *id)?;
    (e.screen_px(zoom) >= MIN_PX).then_some((*id, e))
}

/// The arc of ellipse `id`.
pub fn arc_of(doc: &Document, id: NodeId) -> Option<Arc> {
    Ellipse::of(doc, id).map(|e| e.arc)
}

/// The arc handles of the selection at `zoom` (screen pixels per page unit), if it is one ellipse
/// that shows them.
pub fn handles(doc: &Document, sel: &[NodeId], zoom: f64) -> Option<ArcHandles> {
    let (id, e) = target(doc, sel, zoom)?;
    let a = e.arc;
    let full = a.full();
    let at = |h| pt(e.handle(h, a, zoom));
    Some(ArcHandles {
        id,
        sweep: at(ArcHandle::Sweep),
        start: (!full).then(|| at(ArcHandle::Start)),
        ratio: (!full || a.inner > 0.0).then(|| at(ArcHandle::Ratio)),
    })
}

/// The arc handle under page point `p` (the nearest; Sweep first when two coincide), with its
/// ellipse.
pub fn hit(doc: &Document, sel: &[NodeId], p: Point, zoom: f64) -> Option<(NodeId, ArcHandle)> {
    let hs = handles(doc, sel, zoom)?;
    let tol = HIT_PX / zoom;
    let mut best: Option<(ArcHandle, f64)> = None;
    for h in [ArcHandle::Sweep, ArcHandle::Start, ArcHandle::Ratio] {
        let Some(q) = hs.at(h) else { continue };
        let d = (Point::new(q[0], q[1]) - p).hypot();
        if d <= tol && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((h, d));
        }
    }
    best.map(|(h, _)| (hs.id, h))
}

/// The sweep ending at `d` (radians clockwise from the start, 0..2π) nearest to `prev`: `d` or
/// `d - 2π`. A full turn either way is the same shape, so sweeps live on a circle 4π round and a
/// drag carries on smoothly through ±100%.
fn nearest_sweep(prev: f64, d: f64) -> f64 {
    let dist = |s: f64| {
        let x = (s - prev).rem_euclid(2.0 * TAU);
        x.min(2.0 * TAU - x)
    };
    if dist(d - TAU) < dist(d) { d - TAU } else { d }
}

fn snap(v: f64, step: f64) -> f64 {
    (v / step).round() * step
}

/// The arc after dragging handle `h` of ellipse `id` to page point `p`, from `arc` (the arc so
/// far in this drag, which keeps the sweep continuous). `shift` snaps.
pub fn drag(doc: &Document, id: NodeId, h: ArcHandle, arc: Arc, p: Point, shift: bool, zoom: f64) -> Option<Arc> {
    let e = Ellipse::of(doc, id)?;
    let zoom = sane_zoom(zoom);
    let u = e.unit(p);
    if !(u.x.is_finite() && u.y.is_finite()) {
        return None;
    }
    let angle = u.y.atan2(u.x);
    let mut out = arc;
    match h {
        ArcHandle::Sweep => {
            let sweep = nearest_sweep(arc.sweep, (angle - arc.start).rem_euclid(TAU));
            let sweep = if shift { snap(sweep, SNAP_DEG.to_radians()) } else { sweep };
            // Back at the start (from the far side), it closes.
            let gap = (e.point(arc.start + sweep, 1.0) - e.point(arc.start, 1.0)).hypot() * zoom;
            out.sweep = if sweep.abs() > TAU / 2.0 && gap <= CLOSE_PX { TAU.copysign(sweep) } else { sweep.clamp(-TAU, TAU) };
        }
        ArcHandle::Start => {
            let a = if shift { snap(angle, SNAP_DEG.to_radians()) } else { angle };
            out.start = a.rem_euclid(TAU);
        }
        ArcHandle::Ratio => {
            let r = u.hypot();
            out.inner = (if shift { snap(r, SNAP_RATIO) } else { r }).clamp(0.0, MAX_RATIO);
            // Dragged into the gap (clear of the centre), the other part of the ellipse shows.
            let off_centre = (p - e.point(0.0, 0.0)).hypot() * zoom > HIT_PX;
            if off_centre && !arc.covers(angle) {
                out.start = (arc.start + arc.sweep).rem_euclid(TAU);
                out.sweep = (TAU - arc.sweep.abs()).copysign(arc.sweep);
            }
        }
    }
    [out.start, out.sweep, out.inner].iter().all(|v| v.is_finite()).then_some(out)
}

/// Where handle `h` of ellipse `id` is for `arc` at `zoom` (the readout's anchor while dragging).
pub fn handle_at(doc: &Document, id: NodeId, h: ArcHandle, arc: Arc, zoom: f64) -> Option<[f64; 2]> {
    Ellipse::of(doc, id).map(|e| pt(e.handle(h, arc, sane_zoom(zoom))))
}

/// `arc` as `node.set` props (degrees and a ratio).
pub fn props(arc: Arc) -> serde_json::Map<String, serde_json::Value> {
    serde_json::Map::from_iter([
        ("arcStart".to_string(), serde_json::json!(arc.start.to_degrees())),
        ("arcSweep".to_string(), serde_json::json!(arc.sweep.to_degrees())),
        ("arcInner".to_string(), serde_json::json!(arc.inner)),
    ])
}
