use super::handles::{PENCIL_STROKE, selected_end_at};
use super::{
    Affine, Document, Gesture, Guide, Node, NodeId, NodeKind, PenPath, Point, PointerEvent, Result, Session, StrokeAlign, ToolId, Vec2,
    VectorNetwork, at_45_degrees, frame_at, json, pen_handles,
};
use crate::vectoredit::VTool;

/// Pen pressure along drawn samples as width profile points `[t, scale]`: `t` by length, at most
/// 32 points, medium pressure (0.5) at the stroke weight, from a tenth to twice it. `None` when no
/// sample has pressure.
pub(crate) fn pressure_profile(points: &[Point], pressure: &[Option<f32>]) -> Option<Vec<[f64; 2]>> {
    if pressure.iter().all(Option::is_none) {
        return None;
    }
    let mut lengths = vec![0.0];
    for w in points.windows(2) {
        let (Some(a), Some(b)) = (w.first(), w.get(1)) else { continue };
        lengths.push(lengths.last().copied().unwrap_or(0.0) + (*b - *a).hypot());
    }
    let total = lengths.last().copied().unwrap_or(0.0);
    if !(total.is_finite() && total > 0.0) {
        return None;
    }
    let at = |i: usize| f64::from(pressure.get(i).copied().flatten().filter(|p| p.is_finite()).unwrap_or(0.5).clamp(0.0, 1.0));
    let n = points.len().clamp(2, 32);
    let profile = (0..n).map(|k| {
        let t = k as f64 / (n - 1) as f64;
        let s = t * total;
        let i = lengths.partition_point(|l| *l < s).min(lengths.len().saturating_sub(1));
        let p = match (i.checked_sub(1).and_then(|j| lengths.get(j)), lengths.get(i)) {
            (Some(a), Some(b)) if b > a => at(i - 1) + (at(i) - at(i - 1)) * ((s - a) / (b - a)),
            _ => at(i),
        };
        [t, (p / 0.5).clamp(0.1, 2.0)]
    });
    Some(profile.collect())
}

/// A frame or section drawn around layers takes those of its siblings that lie fully inside it
/// (visible and unlocked ones, in their order; a frame never takes a section). Not in auto layout,
/// where the new frame is one more item of the flow.
pub(super) fn take_enclosed(doc: &mut Document, drawn: NodeId) -> Result<()> {
    let Some(n) = doc.node(drawn) else { return Ok(()) };
    let section = matches!(n.kind, NodeKind::Section { .. });
    let (Some(parent), Some(area)) = (n.parent.and_then(|p| doc.node(p)), doc.world_bounds(drawn)) else { return Ok(()) };
    if parent.layout.is_some() {
        return Ok(());
    }
    let inside: Vec<NodeId> = parent
        .children
        .iter()
        .copied()
        .filter(|c| *c != drawn && doc.node(*c).is_some_and(|k| k.visible && !k.locked && (section || !matches!(k.kind, NodeKind::Section { .. }))))
        .filter(|c| doc.world_bounds(*c).is_some_and(|b| area.contains(b.origin()) && area.contains(Point::new(b.x1, b.y1))))
        .collect();
    for c in inside {
        doc.reparent(c, drawn, None)?;
    }
    Ok(())
}

impl Session {
    /// The pen tool's snapped position for `ev`: its path's vector (or, before it has one, its first
    /// point) counts among the targets.
    pub(super) fn snap_pen_tool(&mut self, ev: &PointerEvent) -> Result<(Point, Vec<Guide>)> {
        let (id, extra) = match &self.tool.pen {
            Some(pen) if pen.id.is_some() => (pen.id, Vec::new()),
            Some(pen) => (None, pen.points.iter().map(|(p, _, _)| *p).collect()),
            None => (None, Vec::new()),
        };
        self.snap_pen(ev, id, &extra)
    }

    pub(super) fn pen_down(&mut self, ev: &PointerEvent) -> Result<()> {
        self.tool.overlay.guides.clear();
        // The segment being placed shows as itself while the press lasts.
        self.tool.overlay.rubber = None;
        self.tool.overlay.rubber_handle = None;
        let p = if ev.shift { ev.p() } else { self.snap_pen_tool(ev)?.0 };
        let (doc, page) = {
            let ds = self.ds()?;
            (ds.doc.clone(), ds.page)
        };
        // With an open path selected, a click on one of its end points continues it in edit mode.
        if self.tool.pen.is_none()
            && let Some((id, v)) = selected_end_at(&doc, &self.selection(), ev.p(), 8.0 / ev.zoom.max(1e-6))
        {
            return self.edit_vector_with(id, VTool::Pen, Some(v));
        }
        let pen = self.tool.pen.get_or_insert_with(PenPath::default);
        // Clicking the first anchor closes the path.
        if pen.points.len() > 2
            && let Some((first, _, _)) = pen.points.first()
            && (*first - p).hypot() * ev.zoom < 8.0
        {
            pen.closed = true;
            self.pen_update()?;
            return self.pen_finish();
        }
        // Clicking another of its own anchors (not the last) connects there and ends the path.
        let near = |q: &Point| (*q - p).hypot() * ev.zoom < 8.0;
        if let Some(k) = (1..pen.points.len().saturating_sub(1)).find(|k| pen.points.get(*k).is_some_and(|(q, _, _)| near(q))) {
            pen.join = Some(k);
            self.pen_update()?;
            return self.pen_finish();
        }
        // A press on the last anchor (a double-click's second press) adds no point on top of it.
        if pen.points.last().is_some_and(|(q, _, _)| near(q)) {
            return Ok(());
        }
        if pen.parent.is_none() {
            pen.parent = Some(frame_at(&doc, page, p, &[]).unwrap_or(page));
        }
        let mut p = p;
        if ev.shift
            && let Some((last, _, _)) = pen.points.last()
        {
            p = at_45_degrees(*last, p);
        }
        pen.points.push((p, Vec2::ZERO, Vec2::ZERO));
        self.tool.gesture = Gesture::PenDrag { anchor: p };
        self.pen_update()
    }

    /// Rebuild the pen's vector node from its anchors.
    pub(super) fn pen_update(&mut self) -> Result<()> {
        let Some(pen) = self.tool.pen.clone() else { return Ok(()) };
        self.tool.overlay.pen = pen.points.iter().map(|(p, _, _)| [p.x, p.y]).collect();
        self.tool.overlay.pen_handles = pen_handles(&pen.points);
        if pen.points.len() < 2 {
            if let Ok(ds) = self.ds_mut() {
                ds.revision += 1;
            }
            return Ok(());
        }
        let page = self.page()?;
        let parent = pen.parent.unwrap_or(page);
        if pen.id.is_none() {
            self.begin_interaction("Pen")?;
        }
        let ds = self.ds_mut()?;
        let doc = ds.doc_mut();
        let pw = doc.world_transform(parent);
        let inv = pw.inverse();
        // Anchors and handles into the parent's space.
        let pts: Vec<(Point, Point, Point)> = pen.points.iter().map(|(p, i, o)| (inv * *p, inv * (*p + *i), inv * (*p + *o))).collect();
        let mut path = figcraft_geom::BezPath::new();
        for (k, (p, _, _)) in pts.iter().enumerate() {
            if k == 0 {
                path.move_to(*p);
            } else if let (Some(prev), Some(cur)) = (pts.get(k - 1), pts.get(k)) {
                path.curve_to(prev.2, cur.1, *p);
            }
        }
        if pen.closed
            && let (Some(last), Some(first)) = (pts.last(), pts.first())
        {
            path.curve_to(last.2, first.1, first.0);
            path.close_path();
        }
        use figcraft_geom::Shape;
        let bb = path.bounding_box();
        path.apply_affine(Affine::translate((-bb.x0, -bb.y0)));
        let mut net = VectorNetwork::from_path(&path);
        // Straight segments stay straight.
        for s in &mut net.segments {
            if (s.ta.0.abs() + s.ta.1.abs()) < 1e-9 && (s.tb.0.abs() + s.tb.1.abs()) < 1e-9 {
                s.ta = (0.0, 0.0);
                s.tb = (0.0, 0.0);
            }
        }
        if !pen.closed {
            net.regions.clear();
        }
        // Anchor k is vertex k of the open path.
        if let (Some(k), Some((last, _, out))) = (pen.join, pts.last()) {
            let from = pts.len() - 1;
            crate::vectoredit::ops::connect(&mut net, from, k, (out.x - last.x, out.y - last.y), (0.0, 0.0));
        }
        let id = match pen.id {
            Some(id) if doc.contains(id) => id,
            _ => {
                let id = doc.alloc_id();
                let mut n = Node::new(id, NodeKind::Vector { network: VectorNetwork::default() }, "Vector", 0.0, 0.0);
                n.fills.clear();
                n.strokes = vec![figcraft_color::Paint::solid(figcraft_color::Color::BLACK)];
                n.stroke.align = StrokeAlign::Center;
                doc.insert(n, parent, None)?;
                id
            }
        };
        let n = doc.get_mut(id)?;
        n.kind = NodeKind::Vector { network: net };
        n.width = bb.width();
        n.height = bb.height();
        n.transform = Affine::translate((bb.x0, bb.y0));
        if pen.closed && n.fills.is_empty() {
            n.fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::DEFAULT_SHAPE)];
        }
        ds.selection = vec![id];
        ds.revision += 1;
        if let Some(p) = self.tool.pen.as_mut() {
            p.id = Some(id);
        }
        Ok(())
    }

    /// Edit `id` as a vector with `tool` (the pen going on from point `anchor`, or, without one,
    /// starting a new path in the same layer at the next click).
    fn edit_vector_with(&mut self, id: NodeId, tool: VTool, anchor: Option<usize>) -> Result<()> {
        self.set_tool(ToolId::Move);
        crate::vectoredit::begin(self, id)?;
        crate::vectoredit::set_tool(self, tool)?;
        crate::vectoredit::set_state(self, |e| {
            e.anchor = anchor;
            e.vertices = anchor.into_iter().collect();
        })
    }

    /// Escape while the pen draws: the path ends but its layer stays in edit mode with the pen, so
    /// the next path joins the same vector (another Escape leaves edit mode).
    pub fn pen_end(&mut self) -> Result<()> {
        self.end_pen_path(VTool::Pen)
    }

    /// Enter (or a double-click) while the pen draws: the path ends and its layer is edited with
    /// the Move tool, its points and handles selectable.
    pub fn pen_enter(&mut self) -> Result<()> {
        self.end_pen_path(VTool::Move)
    }

    /// End the pen's path and edit its layer with `tool` (nothing to edit before a segment).
    fn end_pen_path(&mut self, tool: VTool) -> Result<()> {
        let id = self.tool.pen.as_ref().and_then(|p| p.id);
        self.pen_finish()?;
        match id {
            Some(id) if self.doc()?.contains(id) => self.edit_vector_with(id, tool, None),
            _ => Ok(()),
        }
    }

    /// A double-click with the pen (#1430). While a path is drawn it ends there, as Enter does.
    /// Otherwise the double-click only started a path (its second press adds nothing): that is
    /// dropped, and the layer under the pointer is edited with the Move tool, as a double-click on
    /// it with the Move tool would.
    pub(super) fn pen_double_click(&mut self, ev: &PointerEvent) -> Result<()> {
        if self.tool.pen.as_ref().is_some_and(|p| p.id.is_some()) {
            return self.pen_enter();
        }
        self.pen_finish()?;
        let (doc, page) = {
            let ds = self.ds()?;
            (ds.doc.clone(), ds.page)
        };
        match self.hit(&doc, page, ev.p(), ev.tol() * 2.0).filter(|id| crate::vectoredit::enter_targets(&doc, &[*id]).is_some()) {
            Some(id) => self.edit_vector_with(id, VTool::Move, None),
            None => Ok(()),
        }
    }

    /// Finish the pen path (and keep the pen tool: double-click, Delete, a tool switch).
    pub fn pen_finish(&mut self) -> Result<()> {
        let pen = self.tool.pen.take();
        self.tool.overlay.pen.clear();
        self.tool.overlay.pen_handles.clear();
        self.tool.overlay.rubber = None;
        self.tool.overlay.rubber_handle = None;
        self.tool.gesture = Gesture::None;
        if pen.as_ref().is_some_and(|p| p.id.is_some()) {
            self.commit_interaction()?;
        }
        Ok(())
    }

    pub(super) fn pencil_finish(&mut self, points: &[Point], pressure: &[Option<f32>]) -> Result<()> {
        if points.len() < 2 {
            return Ok(());
        }
        // Smooth: fit a curve through the samples.
        let mut poly = figcraft_geom::BezPath::new();
        for (k, p) in points.iter().enumerate() {
            if k == 0 {
                poly.move_to(*p);
            } else {
                poly.line_to(*p);
            }
        }
        let fitted = figcraft_geom::kurbo::simplify::simplify_bezpath(poly.iter(), 0.75, &Default::default());
        let d = fitted.to_svg();
        if matches!(self.tool.tool, ToolId::Marker | ToolId::Highlighter) {
            return self.ink_finish(&d);
        }
        let page = self.page()?;
        let parent = frame_at(self.doc()?, page, points.first().copied().unwrap_or_default(), &[]).unwrap_or(page);
        let pw = self.doc()?.world_transform(parent);
        let mut local = fitted.clone();
        local.apply_affine(pw.inverse());
        let _ = d;
        let (color, weight) = PENCIL_STROKE;
        let mut props = json!({"fills": [], "strokes": [color.to_css_hex()], "strokeWeight": weight, "strokeCap": "round", "strokeJoin": "round", "strokeAlign": "center"});
        // Pen pressure becomes the stroke's width profile (brushes follow it too).
        if let Some(points) = pressure_profile(points, pressure) {
            props["widthProfile"] = json!(points);
        }
        if let Some(b) = &self.tool.pencil_brush {
            props["brush"] = serde_json::to_value(b).unwrap_or_default();
        }
        self.execute(
            "node.create",
            &json!({"type": "vector", "path": local.to_svg(), "closed": false, "parent": parent, "name": "Vector", "props": props}),
        )?;
        Ok(())
    }
}
