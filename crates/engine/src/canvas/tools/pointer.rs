use super::handles::{handle_at, label_at, locked_in, on_corner_square, resize_cursor, selected_end_at};
use super::{
    Cursor, Document, Gesture, NodeId, NodeKind, Overlay, PenCursor, Point, PointerEvent, PointerKind, Rect, Result, Session, ToolId, at_45_degrees,
    frame_at, json, select_target,
};

impl Session {
    /// The pointer left the canvas: what hovering there showed goes (the distances Alt or Dev Mode
    /// measured and the box measured to, a vector region's highlight), unless a gesture holds it.
    pub fn pointer_left(&mut self) {
        if !self.tool.busy() {
            self.tool.overlay.guides.clear();
            self.tool.overlay.measure_box = None;
        }
        self.vector_pointer_left();
        self.follow_widget_hover(Vec::new());
    }

    /// Show the hover styles of the widget layers `ids` the pointer is over (none: end them), unless
    /// another live preview (a hovered font, say) is showing.
    fn follow_widget_hover(&mut self, ids: Vec<NodeId>) {
        let ours = !self.tool.widget_hover.is_empty() && crate::textpreview::active(self);
        if ids.is_empty() {
            if ours {
                crate::textpreview::end(self);
            }
            self.tool.widget_hover.clear();
            return;
        }
        if ours && self.tool.widget_hover == ids || !ours && crate::textpreview::active(self) {
            return;
        }
        self.tool.widget_hover = if self.widget_hover(&ids).unwrap_or(false) { ids } else { Vec::new() };
    }

    /// What a marquee starting at page point `p` selects inside: the top-level frame it starts in
    /// (its children), or the focus view's frame.
    pub(crate) fn marquee_within(&self, p: Point) -> Result<Option<NodeId>> {
        let (doc, page) = (self.doc()?, self.page()?);
        let within = frame_at(doc, page, p, &[]).filter(|f| doc.node(*f).is_some_and(|n| n.parent == Some(page)));
        let within = within.filter(|f| doc.world_bounds(*f).is_some_and(|b| b.contains(p)));
        Ok(self.ds()?.focused().or(within))
    }

    /// The selection a marquee over page rect `r` makes from `base` (what was selected before):
    /// the layers it touches (`deep`: nested ones too); `additive` (Shift) toggles them.
    pub(crate) fn marquee_selection(&self, r: Rect, within: Option<NodeId>, deep: bool, base: &[NodeId], additive: bool) -> Result<Vec<NodeId>> {
        let (doc, page) = (self.doc()?, self.page()?);
        let mut hits =
            if deep { figcraft_doc::geometry::marquee_deep(doc, page, r) } else { figcraft_doc::geometry::marquee_select(doc, page, r, within) };
        if let Some(f) = self.ds()?.focused() {
            hits.retain(|h| *h != f && doc.is_ancestor(f, *h));
        }
        // A marquee over a frame from outside selects the frame only when it covers it fully.
        if within.is_none() && !deep {
            hits.retain(|h| {
                let full = doc.world_bounds(*h).is_some_and(|b| r.contains(b.origin()) && r.contains(Point::new(b.x1, b.y1)));
                full || !doc.node(*h).is_some_and(|n| n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. }))
            });
        }
        // Shift toggles: what was selected and is touched again comes out.
        let mut sel: Vec<NodeId> = base.iter().copied().filter(|b| !(additive && hits.contains(b))).collect();
        for h in hits {
            if !base.contains(&h) && !sel.contains(&h) {
                sel.push(h);
            }
        }
        Ok(sel)
    }

    pub fn set_tool(&mut self, t: ToolId) {
        if self.tool.tool == ToolId::Pen && t != ToolId::Pen {
            let _ = self.pen_finish();
        }
        if !matches!(t, ToolId::Move | ToolId::Scale | ToolId::Hand) && self.ds().is_ok_and(|d| d.vector_edit.is_some()) {
            let _ = crate::vectoredit::end(self);
        }
        self.tool.tool = t;
        self.tool.gesture = Gesture::None;
        self.tool.pick_only = false;
        // The tool's cursor shows before the pointer moves.
        self.tool.overlay =
            Overlay { builder_remove: self.tool.builder_remove && t == ToolId::ShapeBuilder, cursor: t.cursor(), ..Overlay::default() };
        self.tool.shape_builder = None;
        if t == ToolId::ShapeBuilder {
            // Regions show straight away, before the pointer moves.
            let sel = self.selection();
            let _ = crate::shapebuilder::arrangement(self, &sel);
        }
        if let Ok(ds) = self.ds_mut() {
            if !matches!(t, ToolId::Text | ToolId::TextOnPath | ToolId::Move) {
                ds.text_edit = None;
            }
            ds.revision += 1;
        }
    }

    /// The tool pointer events go to: in Dev Mode every tool but the hand and comments selects.
    fn acting_tool(&self) -> ToolId {
        match self.tool.tool {
            t if self.tool.dev_mode && !matches!(t, ToolId::Hand | ToolId::Comment) => ToolId::Move,
            t => t,
        }
    }

    /// Drop the gesture under way (a drag, a resize, a shape being drawn), restoring the document
    /// as Escape does; the UI calls it when Dev Mode starts, where nothing may move.
    pub fn drop_gesture(&mut self) -> Result<()> {
        self.tool.gesture = Gesture::None;
        self.cancel_interaction()
    }

    /// Feed one pointer event to the current tool.
    pub fn pointer(&mut self, ev: &PointerEvent) -> Result<()> {
        // A held button sends the same move every frame: running the gesture again would only
        // rebuild the same preview (a new document, and a canvas render, every frame).
        if ev.kind != PointerKind::Move {
            self.tool.last_move = None;
        } else if self.tool.busy() {
            if self.tool.last_move.as_ref() == Some(ev) {
                return Ok(());
            }
            self.tool.last_move = Some(ev.clone());
        }
        let tool = self.acting_tool();
        let edits = matches!(tool, ToolId::Move | ToolId::Scale) && !self.tool.dev_mode;
        if edits && self.origin_pointer(ev)? {
            return Ok(());
        }
        if edits && self.ds().is_ok_and(|d| d.vector_edit.is_some()) && self.vector_pointer(ev)? {
            return Ok(());
        }
        if tool == ToolId::ShapeBuilder {
            return self.shape_builder_pointer(ev);
        }
        match ev.kind {
            PointerKind::Hover => self.hover(ev),
            PointerKind::Down => match tool {
                ToolId::Move | ToolId::Scale => {
                    // While a board's vote runs, a click on an object votes.
                    if tool == ToolId::Move && !self.tool.dev_mode && self.vote_down(ev)? {
                        return Ok(());
                    }
                    // A click on the canvas deselects a ruler guide (the UI selects guides itself).
                    self.ds_mut()?.guide = None;
                    self.move_down(ev)
                }
                ToolId::Hand => {
                    let v = &self.ds()?.view;
                    self.tool.gesture = Gesture::Pan { sx: ev.sx, sy: ev.sy, cx: v.cx, cy: v.cy };
                    self.tool.overlay.cursor = Cursor::Grabbing;
                    Ok(())
                }
                ToolId::Text => self.text_down(ev),
                ToolId::TextOnPath => self.text_on_path_down(ev),
                ToolId::Pen => self.pen_down(ev),
                ToolId::Pencil => {
                    self.tool.gesture = Gesture::Pencil { points: vec![ev.p()], pressure: vec![ev.pressure] };
                    Ok(())
                }
                ToolId::Comment => {
                    self.request_ui("comment.new", json!({"x": ev.x, "y": ev.y}));
                    Ok(())
                }
                ToolId::Eyedropper => self.eyedrop(ev),
                t if t.on_boards() => self.board_down(ev),
                t if t.draws_shape() => self.draw_down(ev),
                _ => Ok(()),
            },
            PointerKind::Move => self.drag(ev),
            PointerKind::Up => self.up(ev),
            PointerKind::DoubleClick => self.double_click(ev),
        }
    }

    /// The rotation origin handle while it shows: a press drags it, a double-click puts it back
    /// at the centre. True when the event was the handle's.
    fn origin_pointer(&mut self, ev: &PointerEvent) -> Result<bool> {
        let dragging = matches!(self.tool.gesture, Gesture::Origin);
        if !dragging && !self.ds()?.on_origin_handle(ev.p(), ev.zoom) {
            return Ok(false);
        }
        match ev.kind {
            PointerKind::Down if !dragging => self.tool.gesture = Gesture::Origin,
            PointerKind::Move if dragging => self.drag_origin_to(ev.p(), ev.zoom)?,
            PointerKind::Up if dragging => self.tool.gesture = Gesture::None,
            PointerKind::DoubleClick => self.set_rotation_origin(None)?,
            PointerKind::Hover => {}
            _ => return Ok(false),
        }
        self.tool.overlay.cursor = Cursor::Move;
        Ok(true)
    }

    pub(super) fn hover(&mut self, ev: &PointerEvent) -> Result<()> {
        let tool = self.acting_tool();
        let ds = self.ds()?;
        let (doc, sel, page) = (ds.doc.clone(), ds.selection.clone(), ds.page);
        let mut cursor = tool.cursor();
        // The text being edited, and with the Text tool any text, takes the I-beam: a click there
        // puts the caret.
        let on_edited = ds
            .text_edit
            .as_ref()
            .is_some_and(|te| doc.world_bounds(te.id).is_some_and(|b| b.inflate(ev.tol() * 4.0, ev.tol() * 4.0).contains(ev.p())));
        let on_text =
            |s: &Self| s.hit(&doc, page, ev.p(), ev.tol() * 2.0).and_then(|l| doc.node(l)).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)));
        if tool == ToolId::Text && (on_edited || on_text(self)) {
            cursor = Cursor::Text;
        }
        if tool == ToolId::TextOnPath && (on_edited || self.path_at(&doc, page, ev).is_some()) {
            cursor = Cursor::Text;
        }
        let mut hover = None;
        let mut widget_hover = Vec::new();
        let mut guides = Vec::new();
        let mut measure_box = None;
        let arc_hover = if self.arc_handles_shown() { crate::arc_handles::hit(&doc, &sel, ev.p(), ev.zoom).map(|(_, h)| h) } else { None };
        self.tool.overlay.arc_hover = arc_hover;
        let shape_hover = if self.arc_handles_shown() && !on_corner_square(&doc, &sel, ev.p(), ev.zoom) {
            crate::shape_handles::hit(&doc, &sel, ev.p(), ev.zoom).map(|(_, h)| h)
        } else {
            None
        };
        self.tool.overlay.shape_hover = shape_hover;
        if matches!(tool, ToolId::Move | ToolId::Scale) && on_edited {
            cursor = Cursor::Text;
        } else if matches!(tool, ToolId::Move | ToolId::Scale) && arc_hover.is_none() && shape_hover.is_none() {
            if let Some(h) = handle_at(&doc, &sel, ev.p(), ev.zoom).filter(|_| !self.tool.dev_mode) {
                cursor = resize_cursor(&doc, &sel, h);
            } else {
                let leaf = self.hit(&doc, page, ev.p(), ev.tol() * 2.0);
                if leaf.and_then(|l| crate::widgets::target_at(&doc, l, ev.p())).is_some() {
                    cursor = Cursor::Pointer;
                }
                widget_hover = leaf.map(|l| crate::widgets::hovered_at(&doc, l, ev.p())).unwrap_or_default();
                hover = label_at(&doc, page, &self.tool.labels, ev.p()).or_else(|| leaf.map(|leaf| select_target(&doc, leaf, &sel, ev.cmd)));
                // Alt over a selected layer: a drag from here duplicates it.
                if ev.alt && !self.tool.dev_mode && tool == ToolId::Move && hover.is_some_and(|h| sel.contains(&h) && !locked_in(&doc, h)) {
                    cursor = Cursor::Copy;
                }
                if ev.alt || self.tool.dev_mode {
                    use crate::distance::{Hover, guide_at, hover_lines, padding_frame, parent_of};
                    // Shift+Alt over an auto layout frame: its padding and gaps. Otherwise from the
                    // selection to a ruler guide, the hovered layer or (over empty canvas) the parent.
                    let around: Vec<NodeId> = sel.iter().copied().chain(hover).collect();
                    let what = hover.filter(|_| ev.alt && ev.shift).and_then(|t| padding_frame(&doc, t)).map(Hover::Padding).or_else(|| {
                        if sel.is_empty() {
                            return None;
                        }
                        let guide = guide_at(&doc, page, &around, ev.p(), ev.tol() * 4.0).map(|(vertical, at)| Hover::Guide { vertical, at });
                        guide.or_else(|| hover.or_else(|| parent_of(&doc, page, &sel)).map(Hover::Layer))
                    });
                    if let Some(what) = what {
                        let (g, b) = hover_lines(&doc, page, &sel, what);
                        guides = g;
                        measure_box = b.map(|b| [b.x0, b.y0, b.x1, b.y1]);
                    }
                }
            }
        }
        if tool == ToolId::Move && !self.tool.dev_mode && crate::board::end_handle_at(&doc, &sel, ev.p(), ev.zoom).is_some() {
            cursor = Cursor::Grab;
        }
        self.tool.overlay.measure_box = measure_box;
        self.tool.overlay.cursor = cursor;
        if tool == ToolId::Pen {
            // Where a click would land (snapped), with its guides.
            let (to, g) = if ev.shift { (ev.p(), Vec::new()) } else { self.snap_pen_tool(ev)? };
            guides = g;
            self.tool.overlay.cursor = Cursor::Pen(self.pen_cursor(&doc, &sel, to, ev.zoom));
            if let Some((last, _, out)) = self.tool.pen.as_ref().and_then(|pen| pen.points.last()) {
                // With Shift the band ends where a click puts the point (#970).
                let to = if ev.shift { at_45_degrees(*last, to) } else { to };
                self.tool.overlay.rubber = Some([last.x, last.y, to.x, to.y]);
                self.tool.overlay.rubber_handle = (out.hypot() > 0.0).then_some([last.x + out.x, last.y + out.y]);
            }
        }
        self.tool.overlay.guides = guides;
        if tool == ToolId::Connector {
            self.connector_hover(ev);
        }
        self.follow_widget_hover(widget_hover);
        let ds = self.ds_mut()?;
        if ds.hover != hover {
            ds.hover = hover;
            ds.revision += 1;
        }
        Ok(())
    }

    /// What a click with the Pen tool at `p` (where it snaps) would do: close the path on its first
    /// point, connect to another of its points, go on from a selected open path's end, or add a
    /// new point (the same tests as [`Self::pen_down`]).
    fn pen_cursor(&self, doc: &Document, sel: &[NodeId], p: Point, zoom: f64) -> PenCursor {
        let near = |q: &Point| (*q - p).hypot() * zoom < 8.0;
        let on_point = match &self.tool.pen {
            None => selected_end_at(doc, sel, p, 8.0 / zoom.max(1e-6)).is_some(),
            Some(pen) => {
                let n = pen.points.len();
                (n > 2 && pen.points.first().is_some_and(|(q, _, _)| near(q)))
                    || (1..n.saturating_sub(1)).any(|k| pen.points.get(k).is_some_and(|(q, _, _)| near(q)))
            }
        };
        if on_point { PenCursor::Close } else { PenCursor::New }
    }

    /// Whether what the gesture did to `ids` so far is an edit only a main component can make
    /// (`moved`: their place among the layers counts too): it is undone when the gesture ends.
    pub(super) fn refused(&self, ids: &[NodeId], moved: bool) -> bool {
        let Ok(ds) = self.ds() else { return false };
        ds.interaction.as_ref().is_some_and(|it| {
            ids.iter().any(|m| {
                (moved && !crate::components::move_allowed(&it.doc, &ds.doc, *m))
                    || crate::components::refused_override(&it.doc, &ds.doc, *m).is_some()
            })
        })
    }
}
