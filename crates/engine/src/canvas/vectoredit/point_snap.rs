//! Point snapping: dragged points and handles, and pen clicks, snap to the points and segment
//! midpoints of the vector being edited and of the other vectors on the page, to the usual layer
//! targets (edges and centres of the layers around it, the parent frame, layout grids, guides) and
//! to the pixel grid. Holding Ctrl/Cmd suppresses the object snapping, as when moving layers; the
//! guides (and a marker on a point landed on) go to the overlay like smart guides.

use std::collections::BTreeSet;
use std::sync::Arc;

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_geom::kurbo::ParamCurve;
use figcraft_geom::{Point, Vec2};

use crate::snap::{Guide, Targets};
use crate::tools::{PointerEvent, SNAP_PX};
use crate::{Result, Session};

/// Most points gathered as targets (keeps huge pages responsive).
const MAX_POINTS: usize = 20_000;
/// Most layers visited looking for vectors.
const MAX_NODES: usize = 200_000;
/// Most moving points tested against the targets (the rest move along).
const MAX_MOVING: usize = 256;

/// Snap targets for points of vector `edited` (if any): its points except `moving` (the ones being
/// dragged; their segments' midpoints move too), the points and segment midpoints of the other
/// visible vectors on `page`, the layer targets around it and `extra` page points.
pub fn targets(doc: &Document, page: NodeId, edited: Option<NodeId>, moving: &[usize], extra: &[Point]) -> Targets {
    targets_moving(doc, page, edited, moving, extra, &[])
}

/// [`targets`] while points of other layers move too (`others`: each layer's moving points, which
/// aren't targets).
pub fn targets_moving(
    doc: &Document,
    page: NodeId,
    edited: Option<NodeId>,
    moving: &[usize],
    extra: &[Point],
    others: &[(NodeId, Vec<usize>)],
) -> Targets {
    // Points are snapped on the page.
    let mut t = Targets::gather_in(doc, page, edited.as_slice(), figcraft_geom::Affine::IDENTITY);
    let mut budget = MAX_POINTS;
    for p in extra.iter().take(budget) {
        t.add_point(*p);
    }
    budget = budget.saturating_sub(extra.len());
    if let Some(id) = edited {
        add_vector(doc, id, &moving.iter().copied().collect(), &mut t, &mut budget);
    }
    for id in visible_vectors(doc, page).into_iter().filter(|v| Some(*v) != edited) {
        let skip: BTreeSet<usize> = others.iter().find(|(o, _)| *o == id).map(|(_, v)| v.iter().copied().collect()).unwrap_or_default();
        add_vector(doc, id, &skip, &mut t, &mut budget);
    }
    t
}

/// The visible vectors on `page`, front to back.
fn visible_vectors(doc: &Document, page: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = doc.node(page).map(|n| n.children.clone()).unwrap_or_default();
    let mut visited = 0;
    while let Some(id) = stack.pop() {
        visited += 1;
        if visited > MAX_NODES {
            break;
        }
        let Some(n) = doc.node(id).filter(|n| n.visible) else { continue };
        if matches!(n.kind, NodeKind::Vector { .. }) {
            out.push(id);
        }
        stack.extend(n.children.iter().copied());
    }
    out
}

/// Vector `id`'s points (but `skip`) and the midpoints of segments clear of them, in page space.
fn add_vector(doc: &Document, id: NodeId, skip: &BTreeSet<usize>, t: &mut Targets, budget: &mut usize) {
    let Some(net) = super::network(doc, id) else { return };
    let w = doc.world_transform(id);
    let points = net.vertices.iter().enumerate().filter(|(i, _)| !skip.contains(i)).map(|(_, v)| w * v.point());
    let mids = net
        .segments
        .iter()
        .enumerate()
        .filter(|(_, s)| !skip.contains(&s.a) && !skip.contains(&s.b))
        .filter_map(|(si, _)| Some(w * net.cubic(si)?.eval(0.5)));
    for p in points.chain(mids).filter(|p| p.x.is_finite() && p.y.is_finite()) {
        if *budget == 0 {
            return;
        }
        t.add_point(p);
        *budget -= 1;
    }
}

/// The snapping that applies to a pointer event: (objects and points, the pixel grid).
pub(crate) fn modes(s: &Session, ev: &PointerEvent) -> (bool, bool) {
    let (objects, pixel) = s.ds().map(|d| (d.view.snap_objects, d.view.snap_pixel)).unwrap_or((true, true));
    (objects && !ev.cmd, pixel)
}

/// The correction that snaps page points `pts` (already moved by the pointer) together, and the
/// guides at the result. Object snapping comes first; the axes it leaves free put `grab` (the point
/// under the pointer) on whole pixels. `lock`: the points only move horizontally (`Some(true)`) or
/// vertically (`Some(false)`), so the other axis keeps still.
pub(crate) fn snap(
    t: &Targets,
    pts: &[Point],
    grab: Option<Point>,
    zoom: f64,
    (objects, pixel): (bool, bool),
    lock: Option<bool>,
) -> (Vec2, Vec<Guide>) {
    let pts = pts.get(..MAX_MOVING).unwrap_or(pts);
    let (dx, dy, _) = if objects { t.snap_points(pts, SNAP_PX / zoom.max(1e-6)) } else { (None, None, Vec::new()) };
    let mut d = Vec2::new(dx.unwrap_or(0.0), dy.unwrap_or(0.0));
    if pixel && let Some(g) = grab.filter(|g| g.x.is_finite() && g.y.is_finite()) {
        if dx.is_none() {
            d.x = g.x.round() - g.x;
        }
        if dy.is_none() {
            d.y = g.y.round() - g.y;
        }
    }
    match lock {
        Some(true) => d.y = 0.0,
        Some(false) => d.x = 0.0,
        None => {}
    }
    // The guides show what the points sit on where they end up.
    let guides = if objects {
        let at: Vec<Point> = pts.iter().map(|p| *p + d).collect();
        t.snap_points(&at, 0.0).2
    } else {
        Vec::new()
    };
    (d, guides)
}

/// Targets for a pen (nothing moves), kept while the document, the page, the vector and `extra`
/// stay the same, so hovering doesn't gather them again.
#[derive(Clone, Debug)]
pub struct PenTargets {
    doc: Arc<Document>,
    page: NodeId,
    edited: Option<NodeId>,
    extra: Vec<Point>,
    targets: Arc<Targets>,
}

impl Session {
    /// A pen position: `ev`'s point snapped to the targets around vector `edited` (or the page's,
    /// plus `extra` points: the pen path's own), with the guides to show.
    pub(crate) fn snap_pen(&mut self, ev: &PointerEvent, edited: Option<NodeId>, extra: &[Point]) -> Result<(Point, Vec<Guide>)> {
        let modes = modes(self, ev);
        if modes == (false, false) {
            return Ok((ev.p(), Vec::new()));
        }
        let (doc, page) = {
            let ds = self.ds()?;
            (ds.doc.clone(), ds.page)
        };
        let cached = self.tool.pen_targets.as_ref().filter(|c| Arc::ptr_eq(&c.doc, &doc) && c.page == page && c.edited == edited && c.extra == extra);
        let targets = match cached {
            Some(c) => c.targets.clone(),
            None => {
                let targets = Arc::new(targets(&doc, page, edited, &[], extra));
                self.tool.pen_targets = Some(PenTargets { doc, page, edited, extra: extra.to_vec(), targets: targets.clone() });
                targets
            }
        };
        let (d, guides) = snap(&targets, &[ev.p()], Some(ev.p()), ev.zoom, modes, None);
        Ok((ev.p() + d, guides))
    }
}
