//! The Variable width tool (⇧W) in vector edit mode: width points on the stroke, each with a
//! position along the path and a width. Dragging the path adds a point and sets its width,
//! dragging a point slides it along the path, dragging one of its handles changes its width;
//! Delete removes the selected point. Every gesture is one undo step.
//!
//! **One-sided widths:** with Alt held, dragging a handle (or the path) changes the width on that
//! side of the path only; a plain handle drag makes both sides as wide again. The sides are left
//! and right looking along the path (see [`WidthPoint`]).
//!
//! The points live in the stroke's [`WidthProfile`] and apply to every subpath (positions are
//! fractions of each subpath's length).

use figcraft_doc::{Document, NodeId, WidthProfile, WidthSide};
use figcraft_geom::{Point, Vec2};
use figcraft_pathops::width::{Polyline, nearest, polylines};
use serde::Serialize;
use serde_json::{Value, json};

use super::{editing, set_state};
use crate::cmd::CommandSpec;
use crate::params::{get, num, req_index, req_num, string};
use crate::tools::{Gesture, PointerEvent};
use crate::{Result, Session, bad, spec};

/// Width handles sit at least this far from the path on screen, so thin points stay grabbable.
const HANDLE_MIN_PX: f64 = 8.0;
/// Pointer tolerance (screen pixels).
const TOL_PX: f64 = 6.0;

/// What the pointer is over with the Variable width tool.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum WHit {
    /// A width point on the path (drag slides it along).
    Point { index: usize },
    /// One of a width point's handles (drag sets its width; with Alt, that side's only).
    Handle { index: usize, side: WidthSide },
    /// The path between points (drag adds a point there); `t` is the position along it and `side`
    /// the side of the path the pointer is on.
    Path { t: f64, side: WidthSide },
}

/// A width point as drawn: on the path and its two handles, in page coordinates, per subpath.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidthHandle {
    pub index: usize,
    pub at: [f64; 2],
    /// The handles on the right and the left of the path.
    pub sides: [[f64; 2]; 2],
    /// The stroke's full width there (local units).
    pub width: f64,
    /// Whether the two sides differ.
    pub one_sided: bool,
}

/// An in-progress Variable width gesture.
#[derive(Clone, Copy, Debug)]
pub(crate) enum WidthGesture {
    /// Slide point `index` along the path.
    Slide { index: usize, moved: bool },
    /// Set the width of point `index`; `added` = the gesture added it at `t`.
    Size { index: usize, added: Option<f64>, moved: bool, side: WidthSide },
}

/// The edited node's stroke path flattened (local coordinates) with its weight and profile.
struct Stroked {
    lines: Vec<Polyline>,
    weight: f64,
    profile: Option<WidthProfile>,
}

fn stroked(doc: &Document, id: NodeId) -> Option<Stroked> {
    let n = doc.node(id)?;
    let path = figcraft_doc::geometry::stroke_geometry(n)?;
    Some(Stroked { lines: polylines(&path)?, weight: n.stroke.weight, profile: n.stroke.profile().cloned() })
}

/// Width handles for `(index, t, (left, right) side scales)` on every subpath, as drawn at `zoom`
/// (page coordinates).
fn handles(doc: &Document, id: NodeId, st: &Stroked, pts: impl Iterator<Item = (usize, f64, (f64, f64))> + Clone, zoom: f64) -> Vec<WidthHandle> {
    let w = doc.world_transform(id);
    let min = HANDLE_MIN_PX / zoom.max(1e-6);
    let mut out = Vec::new();
    for line in &st.lines {
        for (index, t, (left, right)) in pts.clone() {
            let Some((at, n)) = line.at(t) else { continue };
            let (hl, hr) = (0.5 * st.weight * left, 0.5 * st.weight * right);
            let c = w * at;
            let dir = w * (at + n) - c;
            let dir = if dir.hypot() > 1e-12 { dir / dir.hypot() } else { Vec2::new(0.0, 1.0) };
            let side = |half: f64, s: f64| {
                let reach = ((w * (at + n * half)) - c).hypot().max(min);
                let q = c + dir * (reach * s);
                [q.x, q.y]
            };
            out.push(WidthHandle {
                index,
                at: [c.x, c.y],
                sides: [side(hr, 1.0), side(hl, -1.0)],
                width: hl + hr,
                one_sided: (left - right).abs() > 1e-9,
            });
        }
    }
    out
}

/// The width points of the vector being edited as drawn at `zoom` (page coordinates).
pub fn width_handles(doc: &Document, ve: &super::VectorEdit, zoom: f64) -> Vec<WidthHandle> {
    let Some(st) = stroked(doc, ve.id) else { return vec![] };
    let Some(profile) = st.profile.clone() else { return vec![] };
    handles(doc, ve.id, &st, profile.points.iter().enumerate().map(|(i, p)| (i, p.t, p.sides())), zoom)
}

/// Where a width point added at `t` would go, with the width there (the hover preview).
pub fn width_preview(doc: &Document, ve: &super::VectorEdit, t: f64, zoom: f64) -> Vec<WidthHandle> {
    let Some(st) = stroked(doc, ve.id) else { return vec![] };
    let sides = st.profile.as_ref().map_or((1.0, 1.0), |p| p.sides_at(t));
    handles(doc, ve.id, &st, std::iter::once((usize::MAX, t, sides)), zoom)
}

/// Hit test the Variable width tool at page point `p`: handles, then points, then the path.
pub fn width_hit(doc: &Document, ve: &super::VectorEdit, p: Point, zoom: f64) -> Option<WHit> {
    let tol = TOL_PX / zoom.max(1e-6);
    let near = |q: [f64; 2]| (Point::new(q[0], q[1]) - p).hypot();
    let handles = width_handles(doc, ve, zoom);
    let best = |pick: &dyn Fn(&WidthHandle) -> f64| {
        handles.iter().map(|h| (h.index, pick(h))).filter(|(_, d)| *d <= tol).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    };
    if let Some(index) = best(&|h| near(h.sides[0]).min(near(h.sides[1]))) {
        // The nearer of its handles (across subpaths).
        let d = |k: usize| handles.iter().filter(|h| h.index == index).map(|h| near(h.sides[k])).fold(f64::INFINITY, f64::min);
        let side = if d(0) <= d(1) { WidthSide::Right } else { WidthSide::Left };
        return Some(WHit::Handle { index, side });
    }
    if let Some(index) = best(&|h| near(h.at)) {
        return Some(WHit::Point { index });
    }
    // Anywhere on the painted stroke counts, or near its centre line when it's thin.
    let st = stroked(doc, ve.id)?;
    let w = doc.world_transform(ve.id);
    let lp = w.inverse() * p;
    let (sub, t, _) = nearest(&st.lines, lp)?;
    let (q, n) = st.lines.get(sub)?.at(t)?;
    let side = if (lp - q).dot(n) >= 0.0 { WidthSide::Right } else { WidthSide::Left };
    let scale = st.profile.as_ref().map_or(1.0, |pr| {
        let (l, r) = pr.sides_at(t);
        if side == WidthSide::Right { r } else { l }
    });
    let half = 0.5 * st.weight * scale * w.determinant().abs().sqrt();
    ((w * q - p).hypot() <= tol.max(half)).then_some(WHit::Path { t, side })
}

/// Write node `id`'s width profile (through `node.set`'s key, so instances record the override).
fn put_profile(doc: &mut Document, id: NodeId, profile: Option<&WidthProfile>) -> Result<()> {
    let v = crate::cmd::props::width_points_json(profile);
    crate::cmd::props::apply("vector.edit.widthPoint", doc, id, &serde_json::Map::from_iter([("widthProfile".to_string(), v)]))
}

/// The node's profile to edit: its own, or uniform with a point at each end.
fn profile_of(doc: &Document, id: NodeId) -> WidthProfile {
    doc.node(id).and_then(|n| n.stroke.profile()).cloned().unwrap_or_else(WidthProfile::uniform)
}

/// Local point `lp` seen from point `t` of the path (the nearest subpath's point there): how far
/// out it is along the normal (negative to the left) and the side it is on.
fn pointer_at(st: &Stroked, t: f64, lp: Point) -> Option<(f64, WidthSide)> {
    let (q, n) = st.lines.iter().filter_map(|l| l.at(t)).min_by(|a, b| (a.0 - lp).hypot().total_cmp(&(b.0 - lp).hypot()))?;
    let along = (lp - q).dot(n);
    Some((along, if along >= 0.0 { WidthSide::Right } else { WidthSide::Left }))
}

/// The width scale for a handle dragged to local point `lp` from point `t` (the nearest
/// subpath's point there counts). With `side`, the scale of that side only: how far the pointer
/// is out to that side (0 past the centre line).
fn scale_at_pointer(st: &Stroked, t: f64, lp: Point, side: Option<WidthSide>) -> Option<f64> {
    let d = match side {
        None => st.lines.iter().filter_map(|l| l.at(t)).map(|(q, _)| (q - lp).hypot()).fold(f64::INFINITY, f64::min),
        Some(s) => {
            let (along, _) = pointer_at(st, t, lp)?;
            (along * if s == WidthSide::Right { 1.0 } else { -1.0 }).max(0.0)
        }
    };
    (d.is_finite() && st.weight > 0.0).then(|| 2.0 * d / st.weight)
}

fn width_point(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "vector.edit.widthPoint";
    let ve = editing(s)?;
    let doc = s.doc()?;
    let weight = doc.get(ve.id)?.stroke.weight;
    let side = match string(p, "side") {
        Some(v) => Some(WidthSide::from_id(&v).ok_or_else(|| bad(cmd, "side: left|right"))?),
        None => None,
    };
    // A side's width runs from the centre line to that edge: half of what both sides make.
    let per_px = if side.is_some() { 2.0 } else { 1.0 };
    let scale = num(p, "scale").or_else(|| num(p, "width").filter(|_| weight > 0.0).map(|w| per_px * w / weight));
    let action = string(p, "action").unwrap_or_else(|| if get(p, "index").is_some() { "set".into() } else { "add".into() });
    let mut prof = profile_of(doc, ve.id);
    if action != "add" && doc.get(ve.id)?.stroke.profile().is_none() {
        return Err(bad(cmd, "the stroke has no width points (add one first)"));
    }
    let selected = match action.as_str() {
        "add" => {
            let t = req_num(cmd, p, "t")?;
            let i = match (scale, side) {
                (Some(sc), None) => prof.insert(t, sc),
                _ => prof.insert_here(t),
            };
            if let (Some(sc), Some(sd)) = (scale, side) {
                prof.set_side(i, sd, sc);
            }
            Some(i)
        }
        "set" | "move" => {
            let i = req_index(cmd, p, "index")?;
            let ok = match side {
                Some(sd) => prof.set_point(i, num(p, "t"), None) && scale.is_none_or(|sc| prof.set_side(i, sd, sc)),
                None => prof.set_point(i, num(p, "t"), scale),
            };
            if !ok {
                return Err(bad(cmd, "no such width point"));
            }
            Some(i)
        }
        "remove" | "delete" => {
            let i = req_index(cmd, p, "index")?;
            if i >= prof.points.len() {
                return Err(bad(cmd, "no such width point"));
            }
            prof.points.remove(i);
            None
        }
        _ => return Err(bad(cmd, "action: add|set|remove")),
    };
    put_profile(s.doc_mut()?, ve.id, Some(&prof))?;
    set_state(s, |e| e.width_point = selected)?;
    Ok(json!({"index": selected, "points": prof.points.len()}))
}

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "vector.edit.widthPoint",
        "Width point",
        "{action?: add|set|remove, index?, t? (0-1 along the path), width? (px) | scale? (× stroke weight), side? (left|right)} — add needs t (width defaults to the current one); set moves or resizes point `index`; with `side` only that side of the path changes (looking along it): `width` is then the distance from the centre line to that edge and `scale` that side's scale (× stroke weight, as if both sides were that wide)",
        true,
        width_point
    )]
}

impl Session {
    /// Hover with the Variable width tool: highlight what a press would grab.
    pub(crate) fn width_hover(&mut self, ve: &super::VectorEdit, ev: &PointerEvent) -> Result<()> {
        self.tool.overlay.width_hover = width_hit(self.doc()?, ve, ev.p(), ev.zoom);
        Ok(())
    }

    /// Press with the Variable width tool.
    pub(crate) fn width_down(&mut self, ve: &super::VectorEdit, ev: &PointerEvent) -> Result<()> {
        let id = ve.id;
        let h = width_hit(self.doc()?, ve, ev.p(), ev.zoom);
        self.tool.overlay.width_hover = None;
        if self.doc()?.get(id)?.stroke.weight <= 0.0 {
            return Ok(());
        }
        let g = match h {
            Some(WHit::Handle { index, side }) => WidthGesture::Size { index, added: None, moved: false, side },
            Some(WHit::Point { index }) => WidthGesture::Slide { index, moved: false },
            Some(WHit::Path { t, side }) => {
                self.begin_interaction("Variable width")?;
                let mut index = 0;
                self.preview(|doc, _| {
                    let mut prof = profile_of(doc, id);
                    index = prof.insert_here(t);
                    put_profile(doc, id, Some(&prof))
                })?;
                WidthGesture::Size { index, added: Some(t), moved: false, side }
            }
            None => {
                return set_state(self, |e| e.width_point = None);
            }
        };
        let (WidthGesture::Slide { index, .. } | WidthGesture::Size { index, .. }) = g;
        set_state(self, |e| e.width_point = Some(index))?;
        self.begin_interaction("Variable width")?;
        self.tool.gesture = Gesture::Vector(super::VGesture::Width(g));
        Ok(())
    }

    /// Drag with the Variable width tool (previewed against the gesture's snapshot).
    pub(crate) fn width_drag(&mut self, ve: &super::VectorEdit, g: WidthGesture, ev: &PointerEvent) -> Result<()> {
        let id = ve.id;
        let lp = self.doc()?.world_transform(id).inverse() * ev.p();
        let g = match g {
            WidthGesture::Slide { index, .. } => {
                self.preview(|doc, _| {
                    let Some(st) = stroked(doc, id) else { return Ok(()) };
                    let Some((_, t, _)) = nearest(&st.lines, lp) else { return Ok(()) };
                    let mut prof = profile_of(doc, id);
                    prof.set_point(index, Some(t), None);
                    put_profile(doc, id, Some(&prof))
                })?;
                WidthGesture::Slide { index, moved: true }
            }
            WidthGesture::Size { index, added, side, .. } => {
                // A handle keeps its side; a point being added widens the side the pointer is on.
                let mut side = side;
                self.preview(|doc, _| {
                    let Some(st) = stroked(doc, id) else { return Ok(()) };
                    let mut prof = profile_of(doc, id);
                    if let Some(t) = added {
                        prof.insert_here(t);
                    }
                    let Some(t) = prof.points.get(index).map(|q| q.t) else { return Ok(()) };
                    if added.is_some()
                        && let Some((_, s)) = pointer_at(&st, t, lp)
                    {
                        side = s;
                    }
                    // Alt: that side only; otherwise both sides as wide.
                    let one_side = ev.alt.then_some(side);
                    let scale = scale_at_pointer(&st, t, lp, one_side);
                    match (one_side, scale) {
                        (Some(s), Some(sc)) => prof.set_side(index, s, sc),
                        _ => prof.set_point(index, None, scale),
                    };
                    put_profile(doc, id, Some(&prof))
                })?;
                WidthGesture::Size { index, added, moved: true, side }
            }
        };
        self.tool.gesture = Gesture::Vector(super::VGesture::Width(g));
        Ok(())
    }

    /// Release: a drag (or a click that added a point) is one undo step.
    pub(crate) fn width_up(&mut self, g: WidthGesture) -> Result<()> {
        match g {
            WidthGesture::Slide { moved: true, .. } | WidthGesture::Size { moved: true, .. } | WidthGesture::Size { added: Some(_), .. } => {
                self.commit_interaction()
            }
            _ => self.cancel_interaction(),
        }
    }
}
