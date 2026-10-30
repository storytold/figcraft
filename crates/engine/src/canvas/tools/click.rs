use super::handles::{PATH_REACH_PX, label_at, locked_in, path_candidates, readable_text_fills};
use super::{
    Affine, Document, Gesture, Node, NodeId, NodeKind, Point, PointerEvent, Rect, Result, Session, Sizing, TextAutoResize, TextData, TextPath,
    ToolId, frame_at, json, select_target, textedit, within_slop,
};

impl Session {
    /// Handle a double-click gesture: finish a pen path, rename or click labels/widgets,
    /// cancel an in-progress move/resize/draw/marquee, handle board double-clicks, enter text
    /// editing (select-all on entry, word-select inside the running edit), enter vector editing
    /// for selected shapes, and step groups/frames one level deeper.
    /// A double-click on `leaf` that a board object or a widget's editable field takes (typing into
    /// a sticky or a shape on a board; editing the field like any text, its plugin told when editing
    /// ends). Whether it was taken.
    fn object_double_click(&mut self, doc: &Document, leaf: NodeId, sel: &[NodeId], p: Point, edits: bool) -> Result<bool> {
        if matches!(self.tool.tool, ToolId::Move | ToolId::Scale) && self.board_double_click(doc, leaf, sel)? {
            return Ok(true);
        }
        match crate::widgets::field_at(doc, leaf, p).filter(|_| edits) {
            Some(field) => textedit::begin(self, field, None, false).map(|()| true),
            None => Ok(false),
        }
    }

    pub(super) fn double_click(&mut self, ev: &PointerEvent) -> Result<()> {
        let ds = self.ds()?;
        let (doc, sel, page) = (ds.doc.clone(), ds.selection.clone(), ds.page);
        if self.tool.tool == ToolId::Pen {
            return self.pen_double_click(ev);
        }
        let edits = !self.tool.dev_mode;
        // A frame's or section's name label: rename it there (the canvas puts a field over it).
        // Labels come before widgets, as on a press; the second press began a move: undo it.
        if edits
            && matches!(self.tool.tool, ToolId::Move | ToolId::Scale)
            && !(ev.shift || ev.cmd || ev.alt)
            && let Some(id) = label_at(&doc, page, &self.tool.labels, ev.p())
        {
            self.tool.gesture = Gesture::None;
            self.cancel_interaction()?;
            if sel != [id] {
                self.set_selection(vec![id]);
            }
            self.request_ui("canvas.renameLabel", serde_json::json!({"id": id}));
            return Ok(());
        }
        // Quick clicks on a widget's clickable part are clicks, one by one.
        if edits
            && matches!(self.tool.tool, ToolId::Move | ToolId::Scale)
            && !(ev.shift || ev.cmd || ev.alt)
            && self.hit(&doc, page, ev.p(), ev.tol() * 2.0).and_then(|hit| crate::widgets::target_at(&doc, hit, ev.p())).is_some()
        {
            self.tool.gesture = Gesture::None;
            self.cancel_interaction()?;
            self.widget_click(ev.p(), ev.tol() * 2.0)?;
            return Ok(());
        }
        // The second press began a move, a resize, a shape or a marquee: a double-click isn't one.
        if matches!(self.tool.gesture, Gesture::Move { .. } | Gesture::Resize { .. } | Gesture::Draw { .. } | Gesture::Marquee { .. }) {
            self.tool.gesture = Gesture::None;
            self.tool.overlay.marquee = None;
            self.cancel_interaction()?;
        }
        let hit_here = self.hit(&doc, page, ev.p(), ev.tol() * 2.0);
        // A double-click anywhere on the one selected shape edits its points, like Enter: also
        // inside a hollow (unfilled) closed shape, where the hit test finds nothing under the
        // pointer (an open path or a line has no inside: only its stroke counts).
        if edits
            && let [id] = sel[..]
            && doc.node(id).is_some_and(|n| n.kind.is_shape() && figcraft_doc::geometry::is_closed(n))
            && doc.world_bounds(id).is_some_and(|b| b.contains(ev.p()))
            && hit_here.is_none_or(|h| h == id || doc.is_ancestor(id, h))
        {
            return crate::vectoredit::begin_many(self, &[id]);
        }
        let Some(leaf) = hit_here else { return Ok(()) };
        if self.object_double_click(&doc, leaf, &sel, ev.p(), edits)? {
            return Ok(());
        }
        let target = select_target(&doc, leaf, &sel, true);
        // Text: edit at the click.
        if edits
            && doc.node(target).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)))
            && (sel.contains(&target) || select_target(&doc, leaf, &sel, false) == target)
        {
            // Entering a text selects all of it; in the text being edited, the word. A
            // double-click inside the running edit is an ordinary edit: starting a new session
            // here would replace the IME owner's epoch and drop a composition on a plain gesture.
            let editing = self.ds()?.text_edit.as_ref().is_some_and(|te| te.id == target);
            let idx = self.text_index_at(target, ev.p())?;
            if editing {
                textedit::select_word(self, idx)?;
            } else {
                textedit::begin(self, target, Some(idx), false)?;
                textedit::key(self, "SelectAll", false, false, false)?;
            }
            return Ok(());
        }
        // Shapes and vectors: enter vector edit mode (every selected shape, the clicked one active).
        // A shape clicked on its own edits even when the click that selects it and the double-click
        // arrived in one frame (a press and a release the canvas took together), so no selection was
        // made yet; another layer's shapes are reached one level at a time, as below.
        if edits && doc.node(target).is_some_and(|n| n.kind.is_shape()) && (sel.contains(&target) || select_target(&doc, leaf, &sel, false) == target)
        {
            let mut ids = if sel.contains(&target) { crate::vectoredit::enter_targets(&doc, &sel).unwrap_or_default() } else { vec![target] };
            ids.retain(|i| *i != target);
            ids.insert(0, target);
            return crate::vectoredit::begin_many(self, &ids);
        }
        // Groups, frames and boolean groups: select one level deeper (the layer under the pointer).
        let deeper = figcraft_doc::geometry::enter_target(&doc, leaf, &sel);
        if deeper != target || !sel.contains(&deeper) {
            self.set_selection(vec![deeper]);
        }
        Ok(())
    }

    /// A plain click at `p` on a clickable part of a widget calls its plugin back (a failure is
    /// shown as a message).
    pub(super) fn widget_click(&mut self, p: Point, tol: f64) -> Result<()> {
        let (doc, page) = (self.doc()?.clone(), self.page()?);
        let Some((id, target)) = self.hit(&doc, page, p, tol).and_then(|hit| crate::widgets::target_at(&doc, hit, p)) else { return Ok(()) };
        if let Err(e) = self.execute("widget.click", &serde_json::json!({"id": id, "target": target})) {
            self.message = Some(e.to_string());
        }
        Ok(())
    }

    /// The char index in text node `id` at page point `p`.
    pub fn text_index_at(&self, id: NodeId, p: Point) -> Result<usize> {
        let doc = self.doc()?;
        let n = doc.get(id)?;
        let local = doc.world_transform(id).inverse() * p;
        let Some((layout, dy)) = figcraft_text::layout_node(n) else { return Ok(0) };
        Ok(layout.hit(local.x, local.y - dy))
    }

    pub(super) fn text_click(&mut self, id: NodeId, ev: &PointerEvent, extend: bool) -> Result<()> {
        let idx = self.text_index_at(id, ev.p())?;
        textedit::set_caret(self, idx, extend)?;
        self.tool.gesture = Gesture::TextSelect { id };
        Ok(())
    }

    pub(super) fn text_down(&mut self, ev: &PointerEvent) -> Result<()> {
        let ds = self.ds()?;
        let (doc, page) = (ds.doc.clone(), ds.page);
        if let Some(te) = ds.text_edit.clone() {
            if doc.world_bounds(te.id).is_some_and(|b| b.contains(ev.p())) {
                return self.text_click(te.id, ev, ev.shift);
            }
            textedit::end(self)?;
        }
        // Clicking existing text edits it.
        if let Some(leaf) = self.hit(&doc, page, ev.p(), ev.tol() * 2.0)
            && doc.node(leaf).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)))
        {
            let idx = self.text_index_at(leaf, ev.p())?;
            textedit::begin(self, leaf, Some(idx), false)?;
            self.tool.gesture = Gesture::TextSelect { id: leaf };
            return Ok(());
        }
        // New text at the click (Figma puts the baseline-ish top-left there).
        let parent = frame_at(&doc, page, ev.p(), &[]).unwrap_or(page);
        let ds = self.ds_mut()?;
        let d = ds.doc_mut();
        let id = d.alloc_id();
        let mut n = Node::new(id, NodeKind::Text(TextData::default()), "Text", 1.0, 14.0);
        let pw = d.world_transform(parent);
        let at = Point::new(ev.x.round(), (ev.y - 7.0).round());
        n.transform = pw.inverse() * Affine::translate(at.to_vec2());
        n.name = String::new();
        if let Some(fills) = readable_text_fills(d, parent) {
            n.fills = fills;
        }
        if let Some((w, h)) = figcraft_text::auto_size(&n) {
            n.width = w.max(1.0);
            n.height = h;
        }
        let line = n.height;
        d.insert(n, parent, None)?;
        ds.selection = vec![id];
        textedit::begin(self, id, Some(0), true)?;
        self.tool.gesture = Gesture::TextBox { id, start: ev.p(), line };
        if !self.prefs.keep_tool {
            self.tool.tool = ToolId::Move;
        }
        Ok(())
    }

    /// The layer whose path is under `ev` for the Text on a path tool and where along it the press
    /// is (0..1 by length): the front-most shape or vector layer within a few screen pixels of its
    /// first subpath (the one text follows), not locked and not inside an instance. Layers in front
    /// whose path is further away don't hide it.
    pub(crate) fn path_at(&self, doc: &Document, page: NodeId, ev: &PointerEvent) -> Option<(NodeId, f64)> {
        let reach = ev.tol() * PATH_REACH_PX;
        let near = |id: NodeId| -> Option<(NodeId, f64)> {
            let n = doc.node(id)?;
            let shape = matches!(
                n.kind,
                NodeKind::Rectangle
                    | NodeKind::Ellipse { .. }
                    | NodeKind::Polygon { .. }
                    | NodeKind::Star { .. }
                    | NodeKind::Line
                    | NodeKind::Vector { .. }
            );
            if !shape || locked_in(doc, id) || crate::components::inside_instance(doc, id) {
                return None;
            }
            let geom = figcraft_doc::geometry::stroke_geometry(n)?;
            let line = figcraft_geom::measure::polylines(&geom)?.into_iter().next()?;
            // Measured on the page (a layer scaled more one way than the other is still reached
            // where it shows), the start by length along the layer's own path.
            let world = doc.world_transform(id);
            let p = ev.p();
            let mut best: Option<(f64, f64)> = None;
            for (i, w) in line.points.windows(2).enumerate() {
                let (a, b) = (world * w[0], world * w[1]);
                let d = b - a;
                let l2 = d.hypot2();
                let u = if l2 > 1e-18 { ((p - a).dot(d) / l2).clamp(0.0, 1.0) } else { 0.0 };
                let dist = (a.lerp(b, u) - p).hypot();
                if best.is_none_or(|(_, bd)| dist < bd) {
                    let (la, lb) = (*line.lengths.get(i)?, *line.lengths.get(i + 1)?);
                    best = Some((la + u * (lb - la), dist));
                }
            }
            let (along, dist) = best?;
            let len = line.length();
            (len > 1e-9 && dist <= reach).then_some((id, along / len))
        };
        let mut candidates = Vec::new();
        path_candidates(doc, page, ev.p(), reach, 0, &mut candidates);
        candidates.into_iter().find_map(near)
    }

    /// A press with the Text on a path tool: on a path, a new empty text layer goes onto it,
    /// starting where it was pressed, and typing starts. The text takes the path layer's place,
    /// geometry, transform, constraints and effects, and its fills when one of them shows; the path
    /// layer is removed. Ending the edit with nothing typed puts the path back without an undo step
    /// ([`textedit::end`]). Elsewhere a press does nothing.
    pub(super) fn text_on_path_down(&mut self, ev: &PointerEvent) -> Result<()> {
        if let Some(te) = self.ds()?.text_edit.clone() {
            if self.ds()?.doc.world_bounds(te.id).is_some_and(|b| b.contains(ev.p())) {
                return self.text_click(te.id, ev, ev.shift);
            }
            textedit::end(self)?;
        }
        let ds = self.ds()?;
        let (doc, page) = (ds.doc.clone(), ds.page);
        let Some((path, start)) = self.path_at(&doc, page, ev) else { return Ok(()) };
        let Some(pn) = doc.node(path) else { return Ok(()) };
        let (Some(parent), Some(geom)) = (pn.parent, figcraft_doc::geometry::stroke_geometry(pn)) else { return Ok(()) };
        let index = doc.index_in_parent(path);
        self.begin_interaction("Text on a path")?;
        let ds = self.ds_mut()?;
        let d = ds.doc_mut();
        let id = d.alloc_id();
        let mut n = Node::new(id, NodeKind::Text(TextData::default()), "Text", pn.width, pn.height);
        n.name = String::new();
        n.transform = pn.transform;
        n.constraints = pn.constraints;
        n.child_layout = pn.child_layout.clone();
        let effects = !pn.effects.is_empty();
        if effects {
            n.effects = pn.effects.clone();
            n.styles.effect = pn.styles.effect.clone();
        }
        // A path drawn with a stroke only has no fill to give: the text gets the usual one.
        let filled = pn.fills.iter().any(|f| f.visible);
        if filled {
            n.fills = pn.fills.clone();
            n.styles.fill = pn.styles.fill.clone();
        } else if let Some(fills) = readable_text_fills(d, parent) {
            n.fills = fills;
        }
        // As `text.onPath`: a binding goes with the list it binds into (`fill`, `fills`, `fills/N/…`).
        let carried = |k: &str| match crate::cmd::lists::list_binding(k) {
            Some((crate::cmd::lists::BoundList::Fills, ..)) => filled,
            Some((crate::cmd::lists::BoundList::Effects, ..)) => effects,
            _ => false,
        };
        n.bound = pn.bound.iter().filter(|(k, _)| carried(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
        if let NodeKind::Text(t) = &mut n.kind {
            let mut tp = TextPath::new(&geom);
            tp.start = start;
            t.path = Some(Box::new(tp));
        }
        d.insert(n, parent, index)?;
        d.remove(path)?;
        ds.selection = vec![id];
        textedit::begin(self, id, Some(0), true)?;
        if !self.prefs.keep_tool {
            self.tool.tool = ToolId::Move;
        }
        Ok(())
    }

    /// Drag out a new text box: past the drag slop it has the drag's size (at least one line
    /// tall) and its top-left, and that size is fixed.
    pub(super) fn text_box_drag(&mut self, ev: &PointerEvent, id: NodeId, start: Point, line: f64) -> Result<()> {
        if within_slop(ev, start) {
            return Ok(());
        }
        let r = Rect::from_points(start, ev.p());
        let d = self.ds_mut()?.doc_mut();
        let pw = d.node(id).and_then(|n| n.parent).map(|p| d.world_transform(p)).unwrap_or_default();
        let n = d.get_mut(id)?;
        n.transform = pw.inverse() * Affine::translate((r.x0.round(), r.y0.round()));
        n.width = r.width().round().max(1.0);
        n.height = r.height().round().max(line);
        n.child_layout.sizing_h = Sizing::Fixed;
        n.child_layout.sizing_v = Sizing::Fixed;
        if let NodeKind::Text(t) = &mut n.kind {
            t.auto_resize = TextAutoResize::None;
        }
        self.ds_mut()?.revision += 1;
        Ok(())
    }

    pub(super) fn eyedrop(&mut self, ev: &PointerEvent) -> Result<()> {
        let ds = self.ds()?;
        let doc = ds.doc.clone();
        let view = figcraft_render::View { origin: (ev.x, ev.y), scale: ev.zoom.max(0.01), width: 1, height: 1 };
        let Some(pix) = figcraft_render::render_view(&doc, ds.page, &view, &Default::default()) else { return Ok(()) };
        let rgba = figcraft_render::to_rgba8(&pix);
        let c = figcraft_color::Color::from_rgba8(
            rgba.first().copied().unwrap_or(0),
            rgba.get(1).copied().unwrap_or(0),
            rgba.get(2).copied().unwrap_or(0),
            255,
        );
        let sel = ds.selection.clone();
        if self.tool.pick_only {
            self.tool.picked = Some(c);
        } else if !sel.is_empty() {
            self.execute("node.set", &json!({"ids": sel, "props": {"fill": c.to_css_hex()}}))?;
        }
        self.recent_colors.retain(|x| *x != c);
        self.recent_colors.insert(0, c);
        self.recent_colors.truncate(16);
        self.message = Some(format!("Picked #{}", c.to_hex()));
        self.set_tool(ToolId::Move);
        Ok(())
    }
}
