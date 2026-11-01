//! Pointer gestures in vector edit mode, one per edit tool. Each gesture is one undo step: it
//! starts an interaction and previews its edit against the snapshot until the pointer goes up.
//! Positions are read in the snapshot's coordinates (the preview re-fits the vector, which moves
//! its origin). Dragged points and handles, and pen clicks, snap (see `point_snap.rs`).

use std::sync::Arc;

use figcraft_doc::{Document, HandleMirroring, NodeId, NodeKind, VectorNetwork, Vertex};
use figcraft_geom::{Affine, Point, Rect, Vec2};

use super::point_snap;
use super::regions::region_hit;
use super::transform::{self, BoxHit, PointBox};
use super::{EditPart, VHit, VTool, VectorEdit, edit, editing, hit, hit_any, local_offset, network, ops, points_of, selected_points, set_state};
use crate::snap::Targets;
use crate::tools::{Cursor, Gesture, Handle, PenCursor, PointerEvent, PointerKind, within_slop};
use crate::{Result, Session};

/// An in-progress edit-mode gesture (cheap to clone: snap targets are shared).
#[derive(Clone, Debug)]
pub(crate) enum VGesture {
    /// Drag points (also a segment's ends) of the active layer and `others`' points with them.
    /// `split` re-adds a point on a segment first; a click without a drag clears the point
    /// selection when `click_clears` (inside the transform box).
    Points {
        start: Point,
        verts: Vec<usize>,
        others: Arc<Vec<(NodeId, Vec<usize>)>>,
        split: Option<(usize, f64)>,
        moved: bool,
        click_clears: bool,
        snap: Arc<PointSnap>,
    },
    /// Drag a handle (`from`: where it was, relative to its point); the other selected handles
    /// (`group`, with where they were) move by as much.
    Handle {
        seg: usize,
        start: bool,
        independent: bool,
        from: (f64, f64),
        group: Arc<Vec<HandleFrom>>,
        targets: Arc<Targets>,
    },
    /// The transform box: a corner or edge scales the points in it. `pick`: a Shift-press on the
    /// point at that corner, which a click (no drag) takes out of the selection instead.
    BoxScale {
        b: PointBox,
        handle: Handle,
        targets: Arc<Targets>,
        pick: Option<usize>,
    },
    /// The transform box: outside a corner rotates the points about its centre (`angle` so far,
    /// degrees clockwise on screen).
    BoxRotate {
        b: PointBox,
        start_angle: f64,
        angle: f64,
    },
    BendSegment {
        seg: usize,
        t: f64,
    },
    /// Bend tool on a point: drag pulls out handles, a click on a smooth point sharpens it.
    BendVertex {
        v: usize,
        origin: Point,
        moved: bool,
        smooth: bool,
    },
    /// Marquee and lasso: `base` is each edited layer's selection to add to. `leave` is a press
    /// on another layer: released without a drag, it leaves edit mode and goes to the
    /// normal tool.
    Marquee {
        start: Point,
        base: Vec<EditPart>,
        leave: Option<PointerEvent>,
    },
    Lasso {
        points: Vec<Point>,
        base: Vec<EditPart>,
    },
    Pen {
        plan: PenPlan,
        origin: Point,
        out: Option<(f64, f64)>,
    },
    /// Cut tool: where the press was (page) and what it was on.
    Cut {
        start: Point,
        hit: Option<VHit>,
    },
    /// Paint bucket: the points dragged over so far (local).
    Paint {
        stamps: Vec<Point>,
    },
    /// Eraser strokes so far (local points) and the brush radius (local units).
    Erase {
        stamps: Vec<Point>,
        radius: f64,
    },
    /// Variable width tool.
    Width(super::width::WidthGesture),
}

/// A handle (segment, the one at its `a` end) and where it was, relative to its point.
type HandleFrom = ((usize, bool), (f64, f64));

/// Snapping for dragged points: their page positions at the start, the one under the pointer (kept
/// on whole pixels) and the targets.
#[derive(Clone, Debug)]
pub(crate) struct PointSnap {
    from: Vec<Point>,
    grab: Option<Point>,
    targets: Targets,
}

/// Eraser brush radius in screen pixels.
const ERASER_PX: f64 = 8.0;

/// A segment's midpoint is a point a press adds when the pointer is this close (screen pixels).
const MIDPOINT_PX: f64 = 6.0;

/// The page point at `t` (0..1) along segment `seg` of vector `id`.
fn segment_point(doc: &Document, id: NodeId, seg: usize, t: f64) -> Option<Point> {
    use figcraft_geom::kurbo::ParamCurve;
    let c = network(doc, id)?.cubic(seg)?;
    Some(doc.world_transform(id) * c.eval(t.clamp(0.0, 1.0)))
}

fn erase_all(net: &mut VectorNetwork, stamps: &[Point], radius: f64) -> bool {
    stamps.iter().fold(false, |acc, c| ops::erase(net, *c, radius) | acc)
}

/// What a pen click does (replayed against the snapshot while the handle is dragged).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PenPlan {
    /// A new point at `at` (local), connected from the pen's open end.
    New { at: Point, from: Option<usize> },
    /// Connect the open end to an existing point (ends the path).
    Connect { from: usize, to: usize },
    /// Add a point on a segment, connected from the open end if any (ends the path).
    Split { seg: usize, t: f64, from: Option<usize> },
}

/// Apply a pen plan; `out` is the open end's outgoing handle, `drag` the handle being dragged out
/// of the new point. Returns the point the click landed on.
fn apply_pen(net: &mut VectorNetwork, plan: PenPlan, out: Option<(f64, f64)>, drag: Option<Vec2>) -> Option<usize> {
    let out = out.unwrap_or((0.0, 0.0));
    let back = drag.map(|d| (-d.x, -d.y)).unwrap_or((0.0, 0.0));
    match plan {
        PenPlan::New { at, from } => {
            net.vertices.push(Vertex::new(at.x, at.y));
            let v = net.vertices.len() - 1;
            if let Some(f) = from {
                ops::connect(net, f, v, out, back);
            }
            if drag.is_some()
                && let Some(x) = net.vertices.get_mut(v)
            {
                x.mirror = Some(HandleMirroring::AngleAndLength);
            }
            Some(v)
        }
        PenPlan::Connect { from, to } => {
            let others = net.incident(to);
            let seg = ops::connect(net, from, to, out, back)?;
            // Dragging while closing shapes the curve on both sides of the point.
            if let (Some(d), [(os, ost)]) = (drag, others.as_slice()) {
                net.set_handle(*os, *ost, (d.x, d.y));
                if let Some(x) = net.vertices.get_mut(to) {
                    x.mirror = Some(HandleMirroring::AngleAndLength);
                }
            }
            let _ = seg;
            Some(to)
        }
        PenPlan::Split { seg, t, from } => {
            let v = ops::split_segment(net, seg, t)?;
            if let Some(f) = from {
                ops::connect(net, f, v, out, (0.0, 0.0));
            }
            Some(v)
        }
    }
}

impl Session {
    /// The transform box to draw around the selected points (page corners tl, tr, br, bl): turned
    /// with them while a rotation is dragged.
    pub fn vector_edit_box(&self) -> Option<[Point; 4]> {
        let ds = self.ds().ok()?;
        if let Gesture::Vector(VGesture::BoxRotate { b, angle, .. }) = &self.tool.gesture {
            let m = figcraft_geom::rotate_about(Affine::IDENTITY, b.center(), -angle);
            return Some(b.corners().map(|c| m * c));
        }
        transform::shown_box(&ds.doc, ds.vector_edit.as_ref()?).map(|b| b.corners())
    }

    /// Feed a pointer event to vector edit mode (the canvas is in edit mode and the main tool is
    /// Move). Returns false when edit mode ended and the event should go to the normal tool.
    pub(crate) fn vector_pointer(&mut self, ev: &PointerEvent) -> Result<bool> {
        let Ok(ve) = editing(self) else { return Ok(false) };
        match ev.kind {
            PointerKind::Hover => self.vector_hover(&ve, ev).map(|()| true),
            PointerKind::Down => self.vector_down(&ve, ev),
            PointerKind::Move => self.vector_drag(&ve, ev).map(|()| true),
            PointerKind::Up => self.vector_up(&ve, ev).map(|()| true),
            PointerKind::DoubleClick if ve.tool == VTool::Pen => self.pen_double_click_editing(ve.id, ev).map(|()| true),
            PointerKind::DoubleClick => {
                let tol = 6.0 / ev.zoom.max(1e-6);
                let doc = self.doc()?.clone();
                if ve.anchor.is_some() {
                    super::escape(self)?;
                } else if hit_any(&doc, &ve, ev.p(), tol).is_none() && ve.tool == VTool::Move {
                    super::end(self)?;
                }
                Ok(true)
            }
        }
    }

    /// A double-click with the pen in edit mode (#1430): the path ends and the layer `id` goes on
    /// being edited with the Move tool. When the double-click began something new (a point split
    /// into a segment, or a new path's first point), that edit of its first press is taken back.
    fn pen_double_click_editing(&mut self, id: NodeId, ev: &PointerEvent) -> Result<()> {
        if let Some((at, depth)) = self.tool.pen_press.take()
            && (at - ev.p()).hypot() * ev.zoom < 8.0
        {
            while self.undo_depth().0 > depth && self.undo()? {}
        }
        if editing(self).is_err() {
            super::begin(self, id)?;
        }
        super::set_tool(self, VTool::Move)
    }

    fn vgesture(&mut self, g: VGesture) {
        self.tool.gesture = Gesture::Vector(g);
    }

    /// The document the gesture started from (its preview is applied to it); the current one
    /// outside gestures.
    fn gesture_doc(&self) -> Result<Arc<Document>> {
        let ds = self.ds()?;
        Ok(ds.interaction.as_ref().map_or_else(|| ds.doc.clone(), |it| it.doc.clone()))
    }

    /// A gesture dragging points `verts` of vector `id` from `start`, and points of other edited
    /// layers with them (positions and snap targets taken from the document as it is now).
    fn points_gesture(
        &self,
        id: NodeId,
        start: Point,
        verts: Vec<usize>,
        others: Vec<(NodeId, Vec<usize>)>,
        split: Option<(usize, f64)>,
        click_clears: bool,
    ) -> Result<VGesture> {
        let ds = self.ds()?;
        let page_points = |id: NodeId, verts: &[usize]| -> Vec<Point> {
            let w = ds.doc.world_transform(id);
            network(&ds.doc, id).map(|n| verts.iter().filter_map(|v| n.vertices.get(*v)).map(|v| w * v.point()).collect()).unwrap_or_default()
        };
        let mut from = page_points(id, &verts);
        for (o, v) in &others {
            from.extend(page_points(*o, v));
        }
        let grab = from.iter().copied().min_by(|a, b| (*a - start).hypot().total_cmp(&(*b - start).hypot()));
        let targets = point_snap::targets_moving(&ds.doc, ds.page, Some(id), &verts, &[], &others);
        Ok(VGesture::Points {
            start,
            verts,
            others: Arc::new(others),
            split,
            moved: false,
            click_clears,
            snap: Arc::new(PointSnap { from, grab, targets }),
        })
    }

    /// A gesture dragging handle `start` of segment `seg` (snapping clear of its own point) and the
    /// handles in `group` with it.
    fn handle_gesture(&self, id: NodeId, seg: usize, start: bool, independent: bool, group: &[(usize, bool)]) -> Result<VGesture> {
        let ds = self.ds()?;
        let net = network(&ds.doc, id);
        let own: Vec<usize> = net.and_then(|n| n.segments.get(seg)).map(|s| vec![if start { s.a } else { s.b }]).unwrap_or_default();
        let targets = Arc::new(point_snap::targets(&ds.doc, ds.page, Some(id), &own, &[]));
        let from = net.and_then(|n| n.handle(seg, start)).unwrap_or_default();
        let group: Vec<HandleFrom> = group.iter().filter(|h| **h != (seg, start)).filter_map(|h| Some((*h, net?.handle(h.0, h.1)?))).collect();
        Ok(VGesture::Handle { seg, start, independent, from, group: Arc::new(group), targets })
    }

    /// Every edited layer's selected points (segment ends included) other than the active one's.
    pub(super) fn others_moving(doc: &Document, ve: &VectorEdit) -> Vec<(NodeId, Vec<usize>)> {
        ve.others
            .iter()
            .filter_map(|o| {
                let v = points_of(network(doc, o.id)?, &o.vertices, &o.segments);
                (!v.is_empty()).then_some((o.id, v))
            })
            .collect()
    }

    /// Each edited layer's selection to add to with Shift (empty selections otherwise).
    fn select_base(ve: &VectorEdit, shift: bool) -> Vec<EditPart> {
        ve.parts().into_iter().map(|p| EditPart { vertices: if shift { p.vertices } else { vec![] }, segments: vec![], id: p.id }).collect()
    }

    /// Select `base` plus the points `found` in each layer (marquee and lasso).
    fn select_parts(&mut self, base: &[EditPart], found: impl Fn(&Document, NodeId) -> Vec<usize>, clear_segments: bool) -> Result<()> {
        let doc = self.doc()?.clone();
        set_state(self, |e| {
            for part in base {
                let mut v = part.vertices.clone();
                for i in found(&doc, part.id) {
                    if !v.contains(&i) {
                        v.push(i);
                    }
                }
                if part.id == e.id {
                    e.vertices = v;
                    if clear_segments {
                        e.segments.clear();
                    }
                } else if let Some(o) = e.others.iter_mut().find(|o| o.id == part.id) {
                    o.vertices = v;
                    if clear_segments {
                        o.segments.clear();
                    }
                }
            }
        })
    }

    /// Start a transform box gesture.
    fn box_down(&mut self, ve: &VectorEdit, bh: BoxHit, ev: &PointerEvent, pick: Option<usize>) -> Result<()> {
        let ds = self.ds()?;
        let (doc, page) = (ds.doc.clone(), ds.page);
        let Some(b) = transform::shown_box(&doc, ve) else { return Ok(()) };
        match bh {
            BoxHit::Inside => {
                let g = self.points_gesture(ve.id, ev.p(), b.points, b.others, None, true)?;
                self.begin_interaction("Move points")?;
                self.vgesture(g);
            }
            BoxHit::Handle(Handle::Rotate(_)) => {
                let start_angle = (ev.p() - b.center()).atan2();
                self.begin_interaction("Rotate points")?;
                self.vgesture(VGesture::BoxRotate { b, start_angle, angle: 0.0 });
            }
            BoxHit::Handle(handle) => {
                let targets = Arc::new(point_snap::targets_moving(&doc, page, Some(ve.id), &b.points, &[], &b.others));
                self.begin_interaction("Scale points")?;
                self.vgesture(VGesture::BoxScale { b, handle, targets, pick });
            }
        }
        Ok(())
    }

    fn vector_hover(&mut self, ve: &VectorEdit, ev: &PointerEvent) -> Result<()> {
        let doc = self.doc()?.clone();
        let tol = 6.0 / ev.zoom.max(1e-6);
        let width_tool = ve.tool == VTool::Width;
        let h = if width_tool { None } else { hit(&doc, ve, ev.p(), tol) };
        // Other edited layers' parts light up too (a press there makes that layer active).
        let shown = if width_tool { None } else { h.map(|h| (ve.id, h)).or_else(|| hit_any(&doc, ve, ev.p(), tol)) };
        // A pen click on empty canvas lands where it snaps.
        let pen_at = if ve.tool == VTool::Pen && h.is_none() && !ev.shift { Some(self.snap_pen(ev, Some(ve.id), &[])?) } else { None };
        // A point a press on the outline would add: the pen's place under its pointer on a segment
        // (its ghost anchor), or the midpoint of the segment under the Move tool.
        let add = match (ve.tool, h) {
            (VTool::Pen, Some(VHit::Segment { seg, t })) => {
                segment_point(&doc, ve.id, seg, t).map(|at| crate::tools::VectorAdd { at: [at.x, at.y], midpoint: false, on: true })
            }
            (VTool::Move, Some(VHit::Segment { seg, .. })) if !ev.cmd && !ev.alt => segment_point(&doc, ve.id, seg, 0.5).map(|at| {
                let on = (at - ev.p()).hypot() * ev.zoom.max(1e-6) <= MIDPOINT_PX;
                crate::tools::VectorAdd { at: [at.x, at.y], midpoint: true, on }
            }),
            _ => None,
        };
        let ov = &mut self.tool.overlay;
        ov.vector_hover = shown.map(|x| x.1);
        ov.vector_hover_on = shown.map(|x| x.0);
        ov.vector_add = add;
        ov.width_hover = None;
        ov.face.clear();
        ov.rubber = None;
        ov.guides.clear();
        ov.brush = (ve.tool == VTool::Eraser).then(|| [ev.x, ev.y, ERASER_PX / ev.zoom.max(1e-6)]);
        // What a press here would do (see `vector_down`).
        ov.cursor = match (ve.tool, h) {
            (VTool::Bend, _) => Cursor::Pen(PenCursor::Bend),
            (VTool::Move, Some(_)) if ev.cmd => Cursor::Pen(PenCursor::Bend),
            (VTool::Move, _) if ev.cmd => Cursor::Crosshair,
            (VTool::Move, Some(VHit::Segment { .. })) if ev.alt => Cursor::Pen(PenCursor::Add),
            // On a segment's midpoint the arrow turns into the pen: a press adds a point there.
            (VTool::Move, Some(VHit::Segment { .. })) if add.is_some_and(|a| a.on) => Cursor::Pen(PenCursor::Add),
            (VTool::Move, _) => Cursor::Default,
            (VTool::Pen, Some(VHit::Vertex { .. })) => Cursor::Pen(PenCursor::Close),
            (VTool::Pen, Some(VHit::Segment { .. })) => Cursor::Pen(PenCursor::Add),
            (VTool::Pen, _) => Cursor::Pen(PenCursor::New),
            _ => Cursor::Crosshair,
        };
        let w = doc.world_transform(ve.id);
        // The Move tool hatches the region under the pointer, clear of anything a press would take
        // first (another layer's parts, Bezier handles, the transform box's handles). Close to a
        // point or segment but outside the region clearances, the region wins (a press takes it).
        let mut region = None;
        match ve.tool {
            VTool::Move if !ev.cmd => {
                // Over the transform box's handles: their cursor, and no point highlighted.
                if let Some(BoxHit::Handle(handle)) = transform::box_hit(&doc, ve, h, ev.p(), ev.zoom)
                    && let Some(b) = transform::shown_box(&doc, ve)
                {
                    self.tool.overlay.cursor = crate::tools::handle_cursor(&b.corners(), handle);
                    self.tool.overlay.vector_hover = None;
                    self.tool.overlay.vector_add = None;
                } else if shown.is_none_or(|(on, h)| on == ve.id && !matches!(h, VHit::Handle { .. })) {
                    region = Self::region_under(&doc, ve, h, ev);
                    if region.is_some() {
                        self.tool.overlay.vector_hover = None;
                        self.tool.overlay.vector_hover_on = None;
                        self.tool.overlay.vector_add = None;
                    }
                }
            }
            VTool::Pen => {
                let (to, guides) = pen_at.unwrap_or((ev.p(), Vec::new()));
                if let Some(a) = ve.anchor.and_then(|a| network(&doc, ve.id)?.vertices.get(a).copied()) {
                    let ap = w * a.point();
                    self.tool.overlay.rubber = Some([ap.x, ap.y, to.x, to.y]);
                }
                self.tool.overlay.guides = guides;
            }
            VTool::Paint => {
                // Highlight the closed area the bucket would fill (skipped on huge networks).
                if let Some(net) = network(&doc, ve.id).filter(|n| n.segments.len() <= 3000)
                    && let Some(face) = net.face_at(w.inverse() * ev.p())
                {
                    let mut polys: Vec<Vec<[f64; 2]>> = Vec::new();
                    let mut path = face.path.clone();
                    path.apply_affine(w);
                    figcraft_geom::kurbo::flatten(path.iter(), 0.25 / ev.zoom.max(1e-6), |el| {
                        use figcraft_geom::kurbo::PathEl;
                        match el {
                            PathEl::MoveTo(p) => polys.push(vec![[p.x, p.y]]),
                            PathEl::LineTo(p) => {
                                if let Some(l) = polys.last_mut() {
                                    l.push([p.x, p.y]);
                                }
                            }
                            _ => {}
                        }
                    });
                    self.tool.overlay.face = polys;
                }
            }
            VTool::Width => self.width_hover(ve, ev)?,
            _ => {}
        }
        let topo = region.and_then(|_| network(&doc, ve.id)).map(VectorNetwork::topology_key);
        if let Ok(ds) = self.ds_mut() {
            if let Some(e) = ds.vector_edit.as_mut()
                && e.region_hover != region
            {
                e.region_hover = region;
                if let Some(k) = topo {
                    e.region_topo = k;
                }
            }
            ds.revision += 1;
        }
        Ok(())
    }

    /// The region of the active layer a press at `ev` would take (Move tool, `h` the network's part
    /// there). Inside the transform box around points selected in their own right (in any edited
    /// layer), the box keeps its drag unless the region is already selected.
    fn region_under(doc: &Document, ve: &VectorEdit, h: Option<VHit>, ev: &PointerEvent) -> Option<usize> {
        let r = region_hit(doc, ve, ev.p(), ev.zoom)?;
        let own = !ve.shown_vertices().is_empty() || !ve.segments.is_empty() || ve.others_selected();
        let in_box = !ev.shift && own && transform::box_hit(doc, ve, h, ev.p(), ev.zoom) == Some(BoxHit::Inside);
        (!in_box || ve.regions.contains(&r)).then_some(r)
    }

    /// A press inside region `r` of the active layer (Move tool): select it with its points (Shift
    /// adds it, or takes it out when it is selected) and drag them. A press on a selected region
    /// keeps the whole selection and drags it.
    fn region_down(&mut self, ve: &VectorEdit, doc: &Document, r: usize, ev: &PointerEvent) -> Result<()> {
        let id = ve.id;
        let Some(net) = network(doc, id) else { return Ok(()) };
        let selected = ve.regions.contains(&r);
        let shift = ev.shift;
        if shift && selected {
            // Its points go too, unless they were selected on their own or another selected
            // region has them.
            let kept: Vec<usize> = ve.regions.iter().filter(|q| **q != r).flat_map(|q| net.region_points(*q)).collect();
            let gone: Vec<usize> = net.region_points(r).into_iter().filter(|v| !kept.contains(v) && ve.region_vertices.contains(v)).collect();
            return set_state(self, |e| {
                e.regions.retain(|q| *q != r);
                e.vertices.retain(|v| !gone.contains(v));
                e.region_vertices.retain(|v| !gone.contains(v));
            });
        }
        if !selected {
            let pts = net.region_points(r);
            let topo = net.topology_key();
            set_state(self, |e| {
                if !shift {
                    e.clear_selection();
                }
                e.handles.clear();
                e.regions.push(r);
                for v in pts {
                    if !e.vertices.contains(&v) {
                        e.vertices.push(v);
                        e.region_vertices.push(v);
                    }
                }
                e.region_topo = topo;
            })?;
        }
        let now = editing(self)?;
        let moving = selected_points(net, &now);
        let others = if shift || selected { Self::others_moving(doc, &now) } else { vec![] };
        let g = self.points_gesture(id, ev.p(), moving, others, None, false)?;
        self.begin_interaction("Move points")?;
        self.vgesture(g);
        Ok(())
    }

    /// Another edited layer under the press (or, with the paint bucket, whose area it is in).
    fn pressed_layer(doc: &Document, ve: &VectorEdit, ev: &PointerEvent, tol: f64) -> Option<NodeId> {
        match hit_any(doc, ve, ev.p(), tol) {
            Some((id, _)) => Some(id),
            None if ve.tool == VTool::Paint => {
                let inside = |id: NodeId| network(doc, id).is_some_and(|n| n.face_at(doc.world_transform(id).inverse() * ev.p()).is_some());
                (!inside(ve.id)).then(|| ve.others.iter().map(|o| o.id).find(|id| inside(*id))).flatten()
            }
            None => None,
        }
    }

    /// Returns false when the click left edit mode (the caller then runs the normal tool).
    fn vector_down(&mut self, ve: &VectorEdit, ev: &PointerEvent) -> Result<bool> {
        let doc = self.doc()?.clone();
        let tol = 6.0 / ev.zoom.max(1e-6);
        // A press on another edited layer's point, handle or segment (or, with the paint bucket,
        // area) makes it the active layer; a pen drawing a path keeps to its own layer.
        let other = Self::pressed_layer(&doc, ve, ev, tol);
        let switched;
        let ve = match other {
            Some(id) if id != ve.id && !(ve.tool == VTool::Pen && ve.anchor.is_some()) => {
                set_state(self, |e| {
                    e.activate(id);
                })?;
                switched = editing(self)?;
                &switched
            }
            _ => ve,
        };
        let h = hit(&doc, ve, ev.p(), tol);
        self.tool.overlay.vector_hover = None;
        self.tool.overlay.vector_add = None;
        self.tool.overlay.face.clear();
        self.tool.overlay.guides.clear();
        let tool = if ev.cmd && ve.tool == VTool::Move { VTool::Bend } else { ve.tool };
        let id = ve.id;
        let w = doc.world_transform(id);
        // Regions come after Bezier handles (and the transform box's handles), before points and
        // segments: clear of those, a press inside one selects it.
        let region = if tool == VTool::Move && !matches!(h, Some(VHit::Handle { .. })) { Self::region_under(&doc, ve, h, ev) } else { None };
        // Any press without Shift drops the selected regions, except one on a selected region
        // (which keeps the selection to drag it); no hover hatch while the button is down.
        let keep_regions = ev.shift || region.is_some_and(|r| ve.regions.contains(&r));
        set_state(self, |e| {
            e.region_hover = None;
            if !keep_regions {
                e.drop_regions();
            }
        })?;
        // The rest of the press sees the selection without them.
        let dropped;
        let ve = if keep_regions || ve.regions.is_empty() {
            ve
        } else {
            dropped = editing(self)?;
            &dropped
        };
        // The transform box (Shift inside it adds to the selection instead; a Shift-click on the
        // point at one of its corners takes that point out, a Shift-drag scales in proportion).
        let pick = match h {
            Some(VHit::Vertex { index }) if ev.shift => Some(index),
            _ => None,
        };
        if tool == VTool::Move
            && let Some(bh) = transform::box_hit(&doc, ve, h, ev.p(), ev.zoom).filter(|bh| !((ev.shift || region.is_some()) && *bh == BoxHit::Inside))
        {
            self.box_down(ve, bh, ev, pick)?;
            return Ok(true);
        }
        if tool == VTool::Move
            && let Some(r) = region
        {
            self.region_down(ve, &doc, r, ev)?;
            return Ok(true);
        }
        match tool {
            VTool::Move => match h {
                Some(VHit::Handle { seg, start }) => {
                    // Shift-click adds or takes out a handle; a press on a selected one drags them all.
                    let key = (seg, start);
                    let handles = if ev.shift {
                        let mut hs = ve.handles.clone();
                        if let Some(i) = hs.iter().position(|x| *x == key) {
                            hs.remove(i);
                        } else {
                            hs.push(key);
                        }
                        hs
                    } else if ve.handles.contains(&key) {
                        ve.handles.clone()
                    } else {
                        vec![key]
                    };
                    let grabbed = handles.contains(&key);
                    let g = self.handle_gesture(id, seg, start, ev.alt, &handles)?;
                    set_state(self, |e| e.handles = handles)?;
                    if grabbed {
                        self.begin_interaction("Edit handle")?;
                        self.vgesture(g);
                    }
                }
                Some(VHit::Vertex { index }) => {
                    // Shift on a point drawn unselected (selected only through a region) selects it
                    // in its own right rather than taking it out.
                    let own = ev.shift && ve.region_vertices.contains(&index);
                    let verts = if own {
                        ve.vertices.clone()
                    } else if ev.shift {
                        let mut v = ve.vertices.clone();
                        if let Some(i) = v.iter().position(|x| *x == index) {
                            v.remove(i);
                        } else {
                            v.push(index);
                        }
                        v
                    } else if ve.vertices.contains(&index) {
                        ve.vertices.clone()
                    } else {
                        vec![index]
                    };
                    let keep_segs = ev.shift;
                    // Shift, or a press on a selected point, keeps (and drags) other layers' points.
                    let keep_others = ev.shift || ve.vertices.contains(&index);
                    set_state(self, |e| {
                        if !keep_others {
                            e.clear_selection();
                        }
                        e.vertices = verts.clone();
                        e.region_vertices.retain(|v| *v != index);
                        e.handles.clear();
                        if !keep_segs {
                            e.segments.clear();
                        }
                    })?;
                    let moving = network(&doc, id)
                        .map(|n| {
                            super::selected_points(
                                n,
                                &VectorEdit { vertices: verts, segments: if keep_segs { ve.segments.clone() } else { vec![] }, ..ve.clone() },
                            )
                        })
                        .unwrap_or_default();
                    let others = if keep_others { Self::others_moving(&doc, ve) } else { vec![] };
                    let g = self.points_gesture(id, ev.p(), moving, others, None, false)?;
                    self.begin_interaction("Move points")?;
                    self.vgesture(g);
                }
                Some(VHit::Segment { seg, t }) => {
                    // A press on the midpoint the hover showed adds a point exactly there; Alt adds
                    // one anywhere else on the segment.
                    let midpoint = !ev.alt
                        && !ev.shift
                        && segment_point(&doc, id, seg, 0.5).is_some_and(|m| (m - ev.p()).hypot() * ev.zoom.max(1e-6) <= MIDPOINT_PX);
                    if ev.alt || midpoint {
                        // Add a point on the segment and drag it.
                        let t = if midpoint { 0.5 } else { t };
                        self.begin_interaction("Add point")?;
                        let mut v = None;
                        self.preview(|doc, _| {
                            v = edit(doc, id, |net| ops::split_segment(net, seg, t))?;
                            Ok(())
                        })?;
                        let Some(v) = v else { return Ok(true) };
                        set_state(self, |e| {
                            e.clear_selection();
                            e.vertices = vec![v];
                        })?;
                        let g = self.points_gesture(id, ev.p(), vec![v], vec![], Some((seg, t)), false)?;
                        self.vgesture(g);
                    } else if ev.shift {
                        set_state(self, |e| {
                            if let Some(i) = e.segments.iter().position(|x| *x == seg) {
                                e.segments.remove(i);
                            } else {
                                e.segments.push(seg);
                            }
                        })?;
                    } else {
                        let ends = network(&doc, id).and_then(|n| n.segments.get(seg)).map(|s| vec![s.a, s.b]).unwrap_or_default();
                        set_state(self, |e| {
                            e.clear_selection();
                            e.segments = vec![seg];
                        })?;
                        let g = self.points_gesture(id, ev.p(), ends, vec![], None, false)?;
                        self.begin_interaction("Move segment")?;
                        self.vgesture(g);
                    }
                }
                None => {
                    // Every drag here marquees (#1508). A click on another layer (and not shift)
                    // leaves edit mode, so the normal tool takes it; a click on empty canvas only
                    // clears the points (a double-click or Escape leaves edit mode).
                    let inside =
                        ve.parts().iter().any(|p| doc.world_render_bounds(p.id).is_some_and(|b| b.inflate(tol * 4.0, tol * 4.0).contains(ev.p())));
                    let page = self.ds()?.page;
                    let other = !inside && self.hit(&doc, page, ev.p(), tol * 2.0).is_some();
                    if !ev.shift {
                        set_state(self, VectorEdit::clear_selection)?;
                    }
                    let leave = (other && !ev.shift).then(|| ev.clone());
                    self.vgesture(VGesture::Marquee { start: ev.p(), base: Self::select_base(ve, ev.shift), leave });
                }
            },
            VTool::Bend => match h {
                Some(VHit::Handle { seg, start }) => {
                    let g = self.handle_gesture(id, seg, start, true, &[])?;
                    self.begin_interaction("Edit handle")?;
                    self.vgesture(g);
                }
                Some(VHit::Vertex { index }) => {
                    let smooth =
                        network(&doc, id).is_some_and(|n| n.incident(index).iter().any(|(s, st)| n.handle(*s, *st).is_some_and(|h| h != (0.0, 0.0))));
                    let origin = network(&doc, id).and_then(|n| n.vertices.get(index)).map(|v| v.point()).unwrap_or_default();
                    set_state(self, |e| {
                        e.clear_selection();
                        e.vertices = vec![index];
                    })?;
                    self.begin_interaction("Bend")?;
                    self.vgesture(VGesture::BendVertex { v: index, origin, moved: false, smooth });
                }
                Some(VHit::Segment { seg, t }) => {
                    self.begin_interaction("Bend")?;
                    self.vgesture(VGesture::BendSegment { seg, t });
                }
                None => self.vgesture(VGesture::Marquee { start: ev.p(), base: Self::select_base(ve, ev.shift), leave: None }),
            },
            VTool::Pen => {
                let at = if h.is_none() && !ev.shift { self.snap_pen(ev, Some(id), &[])?.0 } else { ev.p() };
                let Some(plan) = self.pen_plan(ve, h, w.inverse() * at, ev.shift) else { return Ok(true) };
                self.tool.pen_press = ve.anchor.is_none().then(|| (ev.p(), self.undo_depth().0));
                self.begin_interaction("Pen")?;
                let out = ve.pen_out;
                let mut v = None;
                self.preview(|doc, _| {
                    v = edit(doc, id, |net| apply_pen(net, plan, out, None))?;
                    Ok(())
                })?;
                self.pen_state(plan, v, None)?;
                self.vgesture(VGesture::Pen { plan, origin: ev.p(), out });
            }
            VTool::Paint => {
                // A drag paints every area it crosses the way the first one went (one undo step).
                self.begin_interaction("Paint bucket")?;
                let stamps = vec![w.inverse() * ev.p()];
                let st = stamps.clone();
                self.preview(|doc, _| super::paint_all(doc, id, &st).map(|_| ())).ok();
                self.vgesture(VGesture::Paint { stamps });
            }
            VTool::Eraser => {
                let scale = w.determinant().abs().sqrt().max(1e-9);
                let radius = ERASER_PX / ev.zoom.max(1e-6) / scale;
                let stamps = vec![w.inverse() * ev.p()];
                self.begin_interaction("Eraser")?;
                let st = stamps.clone();
                self.preview(|doc, _| edit(doc, id, |net| erase_all(net, &st, radius)).map(|_| ()))?;
                set_state(self, |e| {
                    e.vertices.clear();
                    e.segments.clear();
                })?;
                self.vgesture(VGesture::Erase { stamps, radius });
            }
            VTool::Width => self.width_down(ve, ev)?,
            VTool::Lasso => {
                self.tool.overlay.lasso = vec![[ev.x, ev.y]];
                self.vgesture(VGesture::Lasso { points: vec![ev.p()], base: Self::select_base(ve, ev.shift) });
            }
            // A click cuts there when the pointer goes up; a drag cuts across (see `cut_up`).
            VTool::Cut => {
                self.tool.overlay.rubber = Some([ev.x, ev.y, ev.x, ev.y]);
                self.vgesture(VGesture::Cut { start: ev.p(), hit: h });
            }
        }
        Ok(true)
    }

    /// What a pen click at local point `lp` does.
    fn pen_plan(&mut self, ve: &VectorEdit, h: Option<VHit>, lp: Point, shift: bool) -> Option<PenPlan> {
        let doc = self.doc().ok()?;
        let net = network(doc, ve.id)?;
        match h {
            Some(VHit::Vertex { index }) => match ve.anchor {
                Some(a) if a != index => Some(PenPlan::Connect { from: a, to: index }),
                Some(_) => {
                    let _ = super::escape(self);
                    None
                }
                None => {
                    // Continue (or branch) from an existing point.
                    let _ = set_state(self, |e| {
                        e.anchor = Some(index);
                        e.pen_out = None;
                        e.vertices = vec![index];
                        e.segments.clear();
                    });
                    None
                }
            },
            Some(VHit::Segment { seg, t }) => Some(PenPlan::Split { seg, t, from: ve.anchor }),
            _ => {
                let mut at = lp;
                // Shift: 45° steps from the open end.
                if shift && let Some(a) = ve.anchor.and_then(|a| net.vertices.get(a)) {
                    let v = lp - a.point();
                    let step = std::f64::consts::FRAC_PI_4;
                    let ang = (v.atan2() / step).round() * step;
                    at = a.point() + Vec2::from_angle(ang) * v.hypot();
                }
                Some(PenPlan::New { at, from: ve.anchor })
            }
        }
    }

    /// Edit state after a pen click.
    fn pen_state(&mut self, plan: PenPlan, v: Option<usize>, out: Option<(f64, f64)>) -> Result<()> {
        set_state(self, |e| {
            e.segments.clear();
            e.vertices = v.into_iter().collect();
            match plan {
                PenPlan::New { .. } => {
                    e.anchor = v;
                    e.pen_out = out;
                }
                PenPlan::Connect { .. } | PenPlan::Split { .. } => {
                    e.anchor = None;
                    e.pen_out = None;
                }
            }
        })
    }

    /// A pen click from a command (`drag` = the handle pulled out of the new point, page units).
    pub(crate) fn pen_click(&mut self, ve: &VectorEdit, p: Point, drag: Vec2, zoom: f64) -> Result<Option<usize>> {
        let doc = self.doc()?.clone();
        let tol = 6.0 / zoom.max(1e-6);
        let w = doc.world_transform(ve.id);
        let Some(plan) = self.pen_plan(ve, hit(&doc, ve, p, tol), w.inverse() * p, false) else {
            return Ok(self.ds()?.vector_edit.as_ref().and_then(|e| e.anchor));
        };
        let inv = w.inverse();
        let ld = inv * (Point::ZERO + drag) - inv * Point::ZERO;
        let drag = (ld.hypot() > 1e-9 && ld.x.is_finite() && ld.y.is_finite()).then_some(ld);
        let v = edit(self.doc_mut()?, ve.id, |net| apply_pen(net, plan, ve.pen_out, drag))?;
        self.pen_state(plan, v, drag.map(|d| (d.x, d.y)))?;
        Ok(v)
    }

    fn vector_drag(&mut self, ve: &VectorEdit, ev: &PointerEvent) -> Result<()> {
        let Gesture::Vector(g) = self.tool.gesture.clone() else { return Ok(()) };
        let id = ve.id;
        let w = self.gesture_doc()?.world_transform(id);
        let modes = point_snap::modes(self, ev);
        let inv = w.inverse();
        let lp = inv * ev.p();
        let to_local = |d: Vec2| inv * (Point::ZERO + d) - inv * Point::ZERO;
        match g {
            VGesture::Points { start, verts, others, split, click_clears, snap, .. } => {
                let mut d = ev.p() - start;
                if d.hypot() * ev.zoom < 1.0 {
                    return Ok(());
                }
                // Shift: along the axis moved most.
                let lock = ev.shift.then_some(d.x.abs() > d.y.abs());
                match lock {
                    Some(true) => d.y = 0.0,
                    Some(false) => d.x = 0.0,
                    None => {}
                }
                let moved: Vec<Point> = snap.from.iter().map(|p| *p + d).collect();
                let (c, guides) = point_snap::snap(&snap.targets, &moved, snap.grab.map(|g| g + d), ev.zoom, modes, lock);
                d += c;
                self.tool.overlay.guides = guides;
                let ld = to_local(d);
                self.preview(|doc, _| {
                    edit(doc, id, |net| {
                        if let Some((seg, t)) = split {
                            ops::split_segment(net, seg, t);
                        }
                        ops::move_vertices(net, &verts, ld);
                    })?;
                    for (o, v) in others.iter() {
                        let od = local_offset(doc.world_transform(*o), d);
                        edit(doc, *o, |net| ops::move_vertices(net, v, od))?;
                    }
                    Ok(())
                })?;
                self.vgesture(VGesture::Points { start, verts, others, split, moved: true, click_clears, snap });
            }
            VGesture::Handle { seg, start, independent, from, group, targets } => {
                let alt = independent || ev.alt;
                let (c, guides) = point_snap::snap(&targets, &[ev.p()], Some(ev.p()), ev.zoom, modes, None);
                self.tool.overlay.guides = guides;
                let lp = inv * (ev.p() + c);
                self.preview(|doc, _| {
                    edit(doc, id, |net| {
                        let Some(s) = net.segments.get(seg).copied() else { return };
                        let Some(v) = net.vertices.get(if start { s.a } else { s.b }).copied() else { return };
                        let rel = (lp.x - v.x, lp.y - v.y);
                        ops::set_handle(net, seg, start, rel, alt);
                        // The other selected handles move as far from where they were.
                        let d = (rel.0 - from.0, rel.1 - from.1);
                        for ((gs, gst), h) in group.iter() {
                            ops::set_handle(net, *gs, *gst, (h.0 + d.0, h.1 + d.1), alt);
                        }
                    })
                })?;
            }
            VGesture::BoxScale { b, handle, targets, .. } => {
                // The dragged corner or edge snaps like a resize handle.
                let (c, guides) = point_snap::snap(&targets, &[ev.p()], None, ev.zoom, (modes.0, false), None);
                self.tool.overlay.guides = guides;
                let q = b.frame.inverse() * (ev.p() + c);
                let (x0, y0, x1, y1) = crate::tools::resized_box(handle, q, b.width, b.height, ev.shift, ev.alt);
                let m = b.onto(x0, y0, x1, y1);
                self.preview(|doc, _| transform::transform_box(doc, id, &b, m))?;
            }
            VGesture::BoxRotate { b, start_angle, .. } => {
                let c = b.center();
                let mut angle = ((ev.p() - c).atan2() - start_angle).to_degrees();
                if ev.shift {
                    angle = (angle / 15.0).round() * 15.0;
                }
                let m = figcraft_geom::rotate_about(Affine::IDENTITY, c, -angle);
                self.preview(|doc, _| transform::transform_box(doc, id, &b, m))?;
                self.tool.overlay.cursor = Cursor::Rotate((ev.p() - c).atan2().to_degrees());
                self.vgesture(VGesture::BoxRotate { b, start_angle, angle });
            }
            VGesture::BendSegment { seg, t } => {
                self.preview(|doc, _| edit(doc, id, |net| ops::bend(net, seg, t, lp)))?;
            }
            VGesture::BendVertex { v, origin, moved, smooth } => {
                let d = lp - origin;
                let page_d = (w * lp - w * origin).hypot() * ev.zoom;
                if !moved && page_d < 2.0 {
                    return Ok(());
                }
                self.preview(|doc, _| edit(doc, id, |net| ops::make_smooth(net, v, d)))?;
                self.vgesture(VGesture::BendVertex { v, origin, moved: true, smooth });
            }
            VGesture::Marquee { start, base, .. } => {
                let r = Rect::from_points(start, ev.p());
                self.tool.overlay.marquee = Some([r.x0, r.y0, r.x1, r.y1]);
                self.select_parts(&base, |doc, id| super::vertices_in(doc, id, r), false)?;
            }
            VGesture::Lasso { mut points, base } => {
                if points.last().is_none_or(|l| (*l - ev.p()).hypot() * ev.zoom >= 2.0) && points.len() < 100_000 {
                    points.push(ev.p());
                }
                self.tool.overlay.lasso = points.iter().map(|p| [p.x, p.y]).collect();
                self.select_parts(&base, |doc, id| super::vertices_in_polygon(doc, id, &points), true)?;
                self.vgesture(VGesture::Lasso { points, base });
            }
            VGesture::Erase { mut stamps, radius } => {
                self.tool.overlay.brush = Some([ev.x, ev.y, ERASER_PX / ev.zoom.max(1e-6)]);
                // Stamps a third of the brush apart, filling gaps on fast strokes.
                let to = lp;
                let from = stamps.last().copied().unwrap_or(to);
                let step = (radius / 3.0).max(1e-6);
                let n = ((to - from).hypot() / step).floor().min(500.0) as usize;
                for k in 1..=n {
                    stamps.push(from.lerp(to, k as f64 / n as f64));
                }
                if n > 0 && stamps.len() < 20_000 {
                    let st = stamps.clone();
                    self.preview(|doc, _| edit(doc, id, |net| erase_all(net, &st, radius)).map(|_| ()))?;
                }
                self.vgesture(VGesture::Erase { stamps, radius });
            }
            VGesture::Cut { start, .. } => self.tool.overlay.rubber = Some([start.x, start.y, ev.x, ev.y]),
            VGesture::Paint { mut stamps } => {
                if stamps.last().is_none_or(|l| (w * *l - ev.p()).hypot() * ev.zoom >= 2.0) && stamps.len() < 10_000 {
                    stamps.push(lp);
                    let st = stamps.clone();
                    self.preview(|doc, _| super::paint_all(doc, id, &st).map(|_| ())).ok();
                }
                self.vgesture(VGesture::Paint { stamps });
            }
            VGesture::Pen { plan, origin, out, .. } => {
                let d = to_local(ev.p() - origin);
                if (ev.p() - origin).hypot() * ev.zoom < 2.0 {
                    return Ok(());
                }
                let mut v = None;
                self.preview(|doc, _| {
                    v = edit(doc, id, |net| apply_pen(net, plan, out, Some(d)))?;
                    Ok(())
                })?;
                self.pen_state(plan, v, Some((d.x, d.y)))?;
                self.vgesture(VGesture::Pen { plan, origin, out });
            }
            VGesture::Width(g) => self.width_drag(ve, g, ev)?,
        }
        Ok(())
    }

    /// The Cut tool's pointer went up: a click on a point or a segment cuts the path there; a drag
    /// cuts every path it crosses, parts that come apart moving to layers of their own.
    fn cut_up(&mut self, ve: &VectorEdit, start: Point, hit: Option<VHit>, ev: &PointerEvent) -> Result<()> {
        self.tool.overlay.rubber = None;
        let id = ve.id;
        self.begin_interaction("Cut")?;
        if (ev.p() - start).hypot() * ev.zoom >= 3.0 {
            return if super::cut_across(self, start, ev.p()).is_ok() { self.commit_interaction() } else { self.cancel_interaction() };
        }
        let mut ends = vec![];
        self.preview(|doc, _| {
            ends = match hit {
                Some(VHit::Vertex { index }) => edit(doc, id, |net| ops::cut_vertex(net, index))?,
                Some(VHit::Segment { seg, t }) => edit(doc, id, |net| ops::cut_segment(net, seg, t))?.map(|(a, b)| vec![a, b]).unwrap_or_default(),
                _ => vec![],
            };
            Ok(())
        })?;
        self.commit_interaction()?;
        set_state(self, |e| {
            e.vertices = ends;
            e.segments.clear();
        })
    }

    fn vector_up(&mut self, ve: &VectorEdit, ev: &PointerEvent) -> Result<()> {
        let g = std::mem::take(&mut self.tool.gesture);
        self.tool.overlay.marquee = None;
        self.tool.overlay.lasso.clear();
        self.tool.overlay.guides.clear();
        let Gesture::Vector(g) = g else { return Ok(()) };
        match g {
            VGesture::Points { verts, others, moved, split, click_clears, .. } => {
                if moved || split.is_some() {
                    if let ([v], None) = (verts.as_slice(), split)
                        && others.iter().all(|(_, o)| o.is_empty())
                    {
                        self.merge_dropped_point(ve.id, *v, 6.0 / ev.zoom.max(1e-6))?;
                    }
                    self.commit_interaction()?;
                } else {
                    self.cancel_interaction()?;
                    if click_clears {
                        self.clear_points()?;
                    }
                }
            }
            VGesture::BendVertex { v, moved, smooth, .. } => {
                if moved {
                    self.commit_interaction()?;
                } else if smooth {
                    let id = ve.id;
                    self.preview(|doc, _| edit(doc, id, |net| ops::make_sharp(net, v)))?;
                    self.commit_interaction()?;
                } else {
                    self.cancel_interaction()?;
                }
            }
            VGesture::Handle { .. } | VGesture::BendSegment { .. } | VGesture::Pen { .. } | VGesture::BoxRotate { .. } => {
                self.commit_interaction()?
            }
            VGesture::Marquee { start, leave: Some(press), .. } if within_slop(ev, start) => {
                super::end(self)?;
                self.pointer(&press)?;
                return self.pointer(ev);
            }
            // The active layer moves to one with points selected when it has none.
            VGesture::Marquee { .. } | VGesture::Lasso { .. } => set_state(self, |e| {
                if e.vertices.is_empty()
                    && let Some(id) = e.others.iter().find(|o| !o.vertices.is_empty()).map(|o| o.id)
                {
                    e.activate(id);
                }
            })?,
            VGesture::Erase { .. } | VGesture::Paint { .. } => self.commit_interaction()?,
            VGesture::BoxScale { pick, .. } => self.box_scale_up(pick)?,
            VGesture::Cut { start, hit } => self.cut_up(ve, start, hit, ev)?,
            VGesture::Width(g) => self.width_up(g)?,
        }
        if let Ok(ds) = self.ds_mut() {
            ds.revision += 1;
        }
        Ok(())
    }

    /// Clears the point selection of the active layer and of the other layers being edited.
    fn clear_points(&mut self) -> Result<()> {
        set_state(self, |e| {
            e.vertices.clear();
            e.segments.clear();
            for o in &mut e.others {
                o.vertices.clear();
                o.segments.clear();
            }
        })
    }

    /// The transform box's corner or edge was let go: a Shift-click (no drag) on the corner's
    /// point `pick` toggles it in the selection; anything else keeps the scale.
    fn box_scale_up(&mut self, pick: Option<usize>) -> Result<()> {
        let ds = self.ds()?;
        let clicked = ds.interaction.as_ref().is_some_and(|it| Arc::ptr_eq(&it.doc, &ds.doc));
        let Some(v) = pick.filter(|_| clicked) else { return self.commit_interaction() };
        self.cancel_interaction()?;
        set_state(self, |e| {
            if let Some(i) = e.vertices.iter().position(|x| *x == v) {
                e.vertices.remove(i);
            } else {
                e.vertices.push(v);
            }
        })
    }

    /// Point `v` of vector `id`, dragged and dropped within `tol` (page units) of another of its
    /// points, merges into that point (in the same undo step as the move), which stays selected.
    fn merge_dropped_point(&mut self, id: NodeId, v: usize, tol: f64) -> Result<()> {
        let moved = self.ds()?.doc.get(id)?.clone();
        let w = self.ds()?.doc.world_transform(id);
        let NodeKind::Vector { network: net } = &moved.kind else { return Ok(()) };
        let Some(p) = net.vertices.get(v).map(|x| w * x.point()) else { return Ok(()) };
        let Some(keep) = net
            .vertices
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != v)
            .map(|(i, x)| (i, (w * x.point() - p).hypot()))
            .filter(|(_, d)| *d <= tol)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
        else {
            return Ok(());
        };
        let mut merged = net.clone();
        ops::merge_vertices(&mut merged, keep, v);
        // A merge that would leave no segment (a 2-point line's ends dropped together) only moves.
        if merged.segments.is_empty() {
            return Ok(());
        }
        // The preview starts again from the document before the drag: bring the moved layer.
        self.preview(|doc, _| {
            *doc.get_mut(id)? = moved;
            edit(doc, id, |net| *net = merged)
        })?;
        set_state(self, |e| {
            e.vertices = vec![if keep > v { keep - 1 } else { keep }];
            e.segments.clear();
        })
    }
}
