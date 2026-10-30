use super::{Document, Gesture, Node, NodeId, Overlay, Point, PointerEvent, PointerKind, Result, Session, ShapeHandle, ToolId, json, textedit};

impl Session {
    /// Keys the current mode handles before menu shortcuts (vector edit mode's tools and point
    /// keys, Enter to finish the shape builder). Returns whether the key was used.
    pub fn tool_key(&mut self, name: &str, shift: bool, alt: bool, cmd: bool) -> Result<bool> {
        // Dev Mode edits nothing: keys go to the commands (which it refuses in the app).
        if self.tool.dev_mode {
            return Ok(false);
        }
        // Enter while the pen draws ends the path and edits it with the Move tool (#1430).
        if name == "Enter" && !alt && !cmd && self.tool.pen.is_some() {
            self.pen_enter()?;
            return Ok(true);
        }
        if self.ds().is_ok_and(|d| d.vector_edit.is_some()) {
            return crate::vectoredit::key(self, name, shift, alt, cmd);
        }
        if !alt && !cmd && self.guide_key(name, shift)? {
            return Ok(true);
        }
        crate::shapebuilder::key(self, name)
    }

    /// Keys for a selected ruler guide: Delete removes it, arrows across it nudge it (Shift: the big
    /// nudge), Escape deselects it.
    fn guide_key(&mut self, name: &str, shift: bool) -> Result<bool> {
        let Some((owner, index)) = self.ds()?.selected_guide() else { return Ok(false) };
        // Backspace is Delete, `Left` is `ArrowLeft`, and so on.
        let name = crate::keymap::normalize(name).map_or(name, |(k, _)| k);
        let page = self.page()?;
        let at = |mut p: serde_json::Value| {
            if owner != page {
                p["frame"] = json!(owner);
            }
            p
        };
        let Some(g) = self.doc()?.node(owner).and_then(|n| n.guides().get(index).cloned()) else { return Ok(false) };
        let step = if shift { self.prefs.nudge_big } else { self.prefs.nudge_small };
        let d = match (name, g.vertical) {
            ("Delete", _) => {
                self.execute("guide.remove", &at(json!({"index": index})))?;
                return Ok(true);
            }
            ("Escape", _) => {
                self.execute("guide.select", &json!({}))?;
                return Ok(true);
            }
            ("ArrowLeft", true) | ("ArrowUp", false) => -step,
            ("ArrowRight", true) | ("ArrowDown", false) => step,
            _ => return Ok(false),
        };
        self.execute("guide.move", &at(json!({"index": index, "offset": g.offset + d})))?;
        Ok(true)
    }

    /// The layer under `p` for a click: the topmost on `page`, or in the focus view the focused
    /// frame's. Boolean groups are hit by their results ([`crate::booleans::result_outline`]); in
    /// outline mode shapes only on their outlines.
    pub fn hit(&self, doc: &Document, page: NodeId, p: Point, tol: f64) -> Option<NodeId> {
        let sel = self.selection();
        let result = crate::booleans::result_outline(doc, &sel);
        let focus = self.ds().ok().and_then(crate::DocState::focused);
        // Outline mode draws no fills: shapes are clicked on their outlines, reaching what's behind.
        if let Some(v) = self.ds().ok().map(|d| &d.view).filter(|v| v.outlines) {
            // What it draws besides the shapes can be clicked too.
            let also = figcraft_doc::geometry::OutlineHits { hidden: v.outline_hidden, bounds: v.outline_bounds };
            return figcraft_doc::geometry::hit_outlines(doc, focus.unwrap_or(page), p, tol, Some(&result), also);
        }
        match focus {
            Some(f) => figcraft_doc::geometry::hit_in(doc, f, p, tol, Some(&result)),
            None => doc.hit_test(page, p, tol, Some(&result)),
        }
    }

    /// Enter with a shape or frame tool (`tool.place`): what a click with it makes, centred on `at`
    /// (default the middle of the view) and selected.
    pub fn place(&mut self, at: Option<Point>) -> Result<()> {
        let tool = self.tool.tool;
        let refuse = |why: &str| Err(crate::EngineError::Disabled("tool.place".into(), why.into()));
        if !tool.draws_shape() {
            return refuse("choose a shape or frame tool first");
        }
        if self.tool.busy() {
            return refuse("finish the drag first");
        }
        let v = &self.ds()?.view;
        let (c, zoom) = (at.unwrap_or(Point::new(v.cx, v.cy)), v.zoom);
        let (w, h) = tool.click_size();
        let ev = |kind| PointerEvent::at(kind, c.x - w / 2.0, c.y - h / 2.0, zoom);
        let r = self.pointer(&ev(PointerKind::Down)).and_then(|()| self.pointer(&ev(PointerKind::Up)));
        if r.is_err() {
            self.tool.gesture = Gesture::None;
            let _ = self.cancel_interaction();
        }
        r
    }

    /// Keyboard Escape: cancel gestures, finish pens, leave text editing, then deselect (in the
    /// focus view, once the focused frame or nothing is selected, it ends the focus view).
    pub fn escape(&mut self) -> Result<()> {
        // Following someone in a live session: Escape stops that first.
        if self.live_unfollow() {
            return Ok(());
        }
        if self.tool.pen.is_some() {
            return self.pen_end();
        }
        if self.tool.busy() {
            self.tool.gesture = Gesture::None;
            self.tool.overlay = Overlay::default();
            return self.cancel_interaction();
        }
        if self.end_origin_edit() {
            return Ok(());
        }
        if self.ds()?.text_edit.is_some() {
            return textedit::end(self);
        }
        if self.ds()?.vector_edit.is_some() {
            return crate::vectoredit::escape(self);
        }
        if self.tool.tool != ToolId::Move {
            self.set_tool(ToolId::Move);
            return Ok(());
        }
        let sel = self.selection();
        if let Some(f) = self.ds()?.focused()
            && (sel.is_empty() || sel == [f])
        {
            self.ds_mut()?.focus = None;
            return Ok(());
        }
        // Escape selects the parent, then nothing.
        let doc = self.doc()?;
        let parents: Vec<NodeId> =
            sel.iter().filter_map(|i| doc.node(*i).and_then(|n| n.parent)).filter(|p| !doc.node(*p).is_some_and(Node::is_page)).collect();
        if !parents.is_empty() {
            let mut uniq = Vec::new();
            for p in parents {
                if !uniq.contains(&p) {
                    uniq.push(p);
                }
            }
            self.set_selection(uniq);
        } else {
            self.set_selection(vec![]);
        }
        Ok(())
    }

    /// Whether the Move tool shows arc and shape handles now (not while another gesture runs, nor
    /// in vector or text edit mode).
    pub(super) fn arc_handles_shown(&self) -> bool {
        let arc_drag = matches!(self.tool.gesture, Gesture::Arc { .. } | Gesture::Shape { .. });
        self.tool.tool == ToolId::Move
            && !self.tool.dev_mode
            && (arc_drag || !self.tool.busy())
            && self.ds().is_ok_and(|ds| ds.vector_edit.is_none() && ds.text_edit.is_none())
    }

    /// The selected ellipse's arc handles as the canvas shows them at the current zoom.
    pub fn arc_handles(&self) -> Option<crate::arc_handles::ArcHandles> {
        let ds = self.ds().ok()?;
        self.arc_handles_shown().then(|| crate::arc_handles::handles(&ds.doc, &ds.selection, ds.view.zoom)).flatten()
    }

    /// The selected shape's corner radius and star ratio handles as the canvas shows them now.
    pub fn shape_handles(&self) -> Vec<(ShapeHandle, [f64; 2])> {
        let Ok(ds) = self.ds() else { return vec![] };
        if self.arc_handles_shown() { crate::shape_handles::handles(&ds.doc, &ds.selection, ds.view.zoom) } else { vec![] }
    }

    /// Snapshot of tool state for the UI / inspect.
    pub fn tool_info(&self) -> serde_json::Value {
        let shapes: Vec<serde_json::Value> = self.shape_handles().into_iter().map(|(h, at)| json!({"handle": h, "at": at})).collect();
        json!({"tool": self.tool.tool.id(), "busy": self.tool.busy(), "overlay": self.tool.overlay, "arcHandles": self.arc_handles(), "shapeHandles": shapes})
    }
}
