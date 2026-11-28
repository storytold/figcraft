use super::handles::{
    SNAP_PX, handle_at, in_quad, label_at, locked_in, on_corner_square, paints_below, resized_box, scale_factor, selection_box, selection_frame,
    shape_hint, shared_rotation_degrees,
};
use super::{
    Affine, ArcHandle, ArcHint, Cursor, Document, Gesture, Guide, Handle, ImageScaleMode, LayoutMode, NodeId, NodeKind, PaintKind, Point,
    PointerEvent, Rect, ResizeStart, Result, ScaleHint, Session, Sizing, Targets, TextAutoResize, ToolId, Vec2, move_world, pencil_add, roots,
    select_target, textedit, within_slop,
};

impl Session {
    pub(super) fn move_down(&mut self, ev: &PointerEvent) -> Result<()> {
        let ds = self.ds()?;
        let (doc, sel, page) = (ds.doc.clone(), ds.selection.clone(), ds.page);
        // Clicking outside the text being edited ends editing.
        if let Some(te) = ds.text_edit.clone() {
            let inside = doc.world_bounds(te.id).is_some_and(|b| b.inflate(ev.tol() * 4.0, ev.tol() * 4.0).contains(ev.p()));
            if inside {
                return self.text_click(te.id, ev, ev.shift);
            }
            textedit::end(self)?;
        }
        // Handles first: a selected connector's ends, a selected ellipse's arc handles (drawn over
        // the box), then the box's.
        let edits = !self.tool.dev_mode;
        if edits && let Some((id, from)) = crate::board::end_handle_at(&doc, &sel, ev.p(), ev.zoom) {
            self.tool.gesture = Gesture::Reattach { id, from, start: ev.p() };
            return Ok(());
        }
        if edits && let Some((id, portion)) = crate::board::path_handle_at(&doc, &sel, ev.p(), ev.zoom) {
            return self.reshape_down(id, portion, ev.p());
        }
        if self.arc_handles_shown()
            && let Some((id, handle)) = crate::arc_handles::hit(&doc, &sel, ev.p(), ev.zoom)
            && let Some(arc) = crate::arc_handles::arc_of(&doc, id)
        {
            self.begin_interaction("Arc")?;
            self.tool.gesture = Gesture::Arc { id, handle, arc };
            self.tool.overlay.arc =
                crate::arc_handles::handle_at(&doc, id, handle, arc, ev.zoom).map(|at| ArcHint { handle, label: arc.label(handle), at });
            return Ok(());
        }
        if self.arc_handles_shown()
            && !on_corner_square(&doc, &sel, ev.p(), ev.zoom)
            && let Some((id, handle)) = crate::shape_handles::hit(&doc, &sel, ev.p(), ev.zoom)
        {
            self.begin_interaction("Corner radius")?;
            self.tool.gesture = Gesture::Shape { id, handle };
            self.tool.overlay.shape = shape_hint(&doc, id, handle, ev.zoom);
            return Ok(());
        }
        if let Some(h) = handle_at(&doc, &sel, ev.p(), ev.zoom).filter(|_| edits) {
            let ids = roots(&doc, &sel);
            if let Handle::Rotate(_) = h {
                let Some(b) = selection_box(&doc, &ids) else { return Ok(()) };
                let center = self
                    .ds()?
                    .moved_origin()
                    .unwrap_or_else(|| Point::new(b.iter().map(|c| c.x).sum::<f64>() / 4.0, b.iter().map(|c| c.y).sum::<f64>() / 4.0));
                self.begin_interaction("Rotate")?;
                self.tool.gesture =
                    Gesture::Rotate { center, start_angle: (ev.p() - center).atan2(), base: shared_rotation_degrees(&doc, &ids), ids };
                return Ok(());
            }
            let nodes: Vec<(NodeId, Affine, f64, f64)> =
                ids.iter().filter_map(|i| doc.node(*i).map(|n| (*i, doc.world_transform(*i), n.width, n.height))).collect();
            let Some(frame) = selection_frame(&doc, &ids) else { return Ok(()) };
            let single = ids.len() == 1;
            self.begin_interaction(if self.tool.tool == ToolId::Scale { "Scale" } else { "Resize" })?;
            self.tool.gesture = Gesture::Resize {
                start: ev.p(),
                r: Box::new(ResizeStart { nodes, frame, single, handle: h }),
                scale_tool: self.tool.tool == ToolId::Scale,
                moved: false,
            };
            return Ok(());
        }
        // A frame's name label picks the frame itself. The background of a top-level frame with
        // layers in it is like the canvas (a press starts a marquee inside it) unless the frame is
        // selected; stepping out from a selected descendant is handled by the marquee below.
        // An empty frame (or a widget, or a board object such as a sticky) is picked and dragged.
        let in_section = |n: &figcraft_doc::Node| n.parent.and_then(|p| doc.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Section { .. }));
        let board_background = |leaf: NodeId| {
            (ev.cmd || !sel.contains(&leaf))
                && doc.node(leaf).is_some_and(|n| {
                    let board = match n.kind {
                        NodeKind::Section { contents_hidden } => !contents_hidden,
                        ref k => k.is_frame_like() && (n.parent == Some(page) || in_section(n)),
                    };
                    board && !n.children.is_empty() && !crate::widgets::is_widget(n) && !n.is_board_atom()
                })
        };
        let label = label_at(&doc, page, &self.tool.labels, ev.p());
        let leaf = self.hit_pick(&doc, page, ev.p(), ev.tol() * 2.0, ev.cmd);
        let hit = label.or(leaf);
        let background_under_cmd = leaf.filter(|l| ev.cmd && label.is_none() && board_background(*l));
        let target = label.or_else(|| leaf.filter(|leaf| !board_background(*leaf)).map(|leaf| select_target(&doc, leaf, &sel, ev.cmd)));
        // A press inside the selection's box that reaches nothing above it (an unfilled shape's
        // inside, the frame or canvas behind it, or a layer stacked below the selected layer the
        // press is in) drags the selection. In a gap between selected layers, what's there is hit.
        let in_own_box = |s: NodeId| selection_box(&doc, &[s]).is_some_and(|q| in_quad(&q, ev.p()));
        // A press on a layer inside a selected one (a child of a selected frame) drags the
        // selection too, and a click selects that layer; Cmd/Ctrl keeps the deep pick.
        let in_selected = |t: NodeId| sel.iter().any(|s| *s != t && doc.is_ancestor(*s, t));
        // With Cmd/Ctrl, a selected nested frame the deep pick went through here (no fill under the
        // pointer, see `HitOpts::deep`) isn't pressed on: the layer under it is the pick, as when
        // the frame isn't selected.
        let passed_through =
            |s: NodeId| ev.cmd && doc.node(s).is_some_and(|n| n.kind.is_frame_like()) && !hit.is_some_and(|h| h == s || doc.is_ancestor(s, h));
        let behind_sel = |t: Option<NodeId>| match t {
            // Reaching nothing counts only for a selected layer the press wasn't passed through.
            None => sel.iter().any(|s| !passed_through(*s)),
            Some(t) => {
                (!ev.cmd && in_selected(t))
                    || sel.iter().any(|s| *s != t && !passed_through(*s) && (doc.is_ancestor(t, *s) || (in_own_box(*s) && paints_below(&doc, t, *s))))
            }
        };
        // What the press hit (a label or a painted layer): a click that doesn't become a drag
        // selects just it. A press that reaches nothing above the selection uses the selection as
        // its drag target instead; that fallback is not a clicked layer.
        let clicked = target;
        let marks_out_a_selected_board = background_under_cmd.is_some_and(|b| sel.contains(&b));
        let target = match sel.iter().copied().find(|s| !locked_in(&doc, *s)) {
            Some(first)
                if label.is_none()
                    && !ev.shift
                    && !marks_out_a_selected_board
                    && behind_sel(target)
                    && selection_box(&doc, &sel).is_some_and(|q| in_quad(&q, ev.p())) =>
            {
                Some(first)
            }
            _ => target,
        };
        match target {
            Some(target) => {
                // Clicking inside the current selection's box keeps it (drag them all); a click
                // that doesn't become a drag narrows it to the clicked layer.
                let in_sel_box = sel.len() > 1 && doc.bounds_of(&sel).is_some_and(|b| b.contains(ev.p())) && !ev.shift;
                // Shift on a selected layer takes it out on release if the press doesn't become a
                // drag; a drag moves the selection with it. Where no drag can start, it goes now.
                let deselect = (ev.shift && sel.contains(&target)).then_some(target);
                let without_target = || sel.iter().copied().filter(|x| *x != target).collect::<Vec<_>>();
                // A release without a drag narrows the selection to the layer the press hit. A
                // selected frame's name label sits outside the selection bounds, so it counts too.
                // A layer inside a selected one counts as well: the press drags the selection, and
                // a click selects that layer.
                let keep = clicked.filter(|c| {
                    !ev.cmd
                        && (((label.is_some() || in_sel_box) && (sel.contains(c) || sel.iter().any(|s| doc.is_ancestor(*s, *c)))) || in_selected(*c))
                });
                let new_sel = if keep.is_some() {
                    sel.clone()
                } else if ev.shift && deselect.is_none() {
                    let mut s2 = sel.clone();
                    s2.push(target);
                    s2
                } else if sel.contains(&target) {
                    sel.clone()
                } else {
                    vec![target]
                };
                self.set_selection(new_sel.clone());
                // Dev Mode only selects (Shift-click still deselects): nothing moves. No move
                // gesture starts, so a click selects the layer it hit now instead of on release.
                if !edits {
                    if deselect.is_some() {
                        self.set_selection(without_target());
                    } else if let Some(k) = keep.filter(|k| sel.as_slice() != [*k]) {
                        self.set_selection(vec![k]);
                    }
                    return Ok(());
                }
                // Locked layers selected in the layers panel stay put.
                let ids: Vec<NodeId> = roots(&doc, &new_sel).into_iter().filter(|i| !locked_in(&doc, *i)).collect();
                if new_sel.is_empty() || doc.node(target).is_some_and(|n| n.locked) || ids.is_empty() {
                    if deselect.is_some() {
                        self.set_selection(without_target());
                    } else if let Some(k) = keep {
                        // No drag can start here, so the click narrows the selection now.
                        self.set_selection(vec![k]);
                    }
                    return Ok(());
                }
                let mut targets = Targets::gather(&doc, page, &ids);
                // A copy dragged out with Alt snaps to the layers it came from too (they stay put).
                if ev.alt
                    && let Some(b) = targets.moving
                {
                    targets.add_box(b);
                }
                if let Err(e) = self.begin_interaction(if ev.alt { "Duplicate" } else { "Move" }) {
                    if deselect.is_some() {
                        self.set_selection(without_target());
                    }
                    return Err(e);
                }
                self.tool.gesture = Gesture::Move { start: ev.p(), ids, targets, duplicate: ev.alt, moved: false, deselect, keep };
                self.tool.overlay.cursor = if ev.alt { Cursor::Copy } else { Cursor::Default };
            }
            None => {
                let within = self.marquee_within(ev.p())?;
                // Clicking the background of the frame containing the current selection steps
                // out to that frame. Keep the marquee gesture (without a Move interaction), so
                // dragging here still selects its contents and cannot leave a stale snapshot.
                let parent = hit.filter(|f| !ev.shift && !ev.cmd && within == Some(*f) && sel.iter().any(|s| doc.is_ancestor(*f, *s)));
                if let Some(f) = parent {
                    self.set_selection(vec![f]);
                } else if !ev.shift {
                    self.set_selection(background_under_cmd.into_iter().collect());
                }
                self.tool.gesture = Gesture::Marquee {
                    start: ev.p(),
                    additive: ev.shift,
                    base: if ev.shift { sel } else { vec![] },
                    within,
                    pending_click: parent.is_some(),
                };
            }
        }
        Ok(())
    }

    pub(super) fn drag(&mut self, ev: &PointerEvent) -> Result<()> {
        let mut g = std::mem::take(&mut self.tool.gesture);
        // Alt makes a move a copy whenever it goes down, before the drag or during it, and letting
        // go of it moves the original again; a copy snaps to the layers it came from too.
        if let Gesture::Move { targets, duplicate, .. } = &mut g
            && *duplicate != ev.alt
        {
            *duplicate = ev.alt;
            if let Some(b) = targets.moving {
                if ev.alt { targets.add_box(b) } else { targets.remove_box(b) }
            }
            if let Some(it) = self.ds_mut().ok().and_then(|ds| ds.interaction.as_mut()) {
                it.label = if ev.alt { "Duplicate" } else { "Move" }.into();
            }
        }
        if let Gesture::Marquee { start, pending_click, .. } = &mut g
            && *pending_click
        {
            if within_slop(ev, *start) {
                self.tool.gesture = g;
                return Ok(());
            }
            *pending_click = false;
        }
        let res = match &g {
            Gesture::None => return self.hover(ev),
            Gesture::Marquee { start, additive, base, within, .. } => {
                let r = Rect::from_points(*start, ev.p());
                self.tool.overlay.marquee = Some([r.x0, r.y0, r.x1, r.y1]);
                // Cmd/Ctrl: the nested layers it touches.
                let sel = self.marquee_selection(r, *within, ev.cmd, base, *additive)?;
                self.set_selection(sel);
                Ok(())
            }
            Gesture::Move { start, moved: false, .. } | Gesture::Resize { start, moved: false, .. } if within_slop(ev, *start) => Ok(()),
            Gesture::Move { start, ids, targets, duplicate, .. } => {
                let mut d = ev.p() - *start;
                if ev.shift {
                    if d.x.abs() > d.y.abs() {
                        d.y = 0.0;
                    } else {
                        d.x = 0.0;
                    }
                }
                // Cmd/Ctrl (and Control on macOS) held while dragging suspends snapping.
                // The pointer's own drag, before snapping: a reorder in auto layout follows it.
                let raw = d;
                let snap_on = self.ds().map(|ds| ds.view.snap_objects).unwrap_or(true) && !ev.cmd && !ev.ctrl;
                let mut guides = Vec::new();
                let boards = {
                    let doc = self.doc()?;
                    ids.iter().all(|i| doc.node(*i).is_some_and(crate::snap::always_on_pixels))
                };
                if boards || self.ds().map(|ds| ds.view.snap_pixel).unwrap_or(true) {
                    // Keep the box's origin on whole pixels (a rotated parent frame's, inside one);
                    // snapping to layers comes after, so it wins on the axes it snaps.
                    d = targets.round_move(d);
                }
                if snap_on {
                    let (c, g2) = targets.snap_move(d, SNAP_PX / ev.zoom.max(1e-6));
                    d += c;
                    guides = g2;
                }
                if boards {
                    // Frames, sections and components stay on whole pixels even where a layer
                    // they snap to doesn't.
                    d = targets.round_move(d);
                }
                let ids = ids.clone();
                let dup = *duplicate;
                let page = self.page()?;
                let mut drop_cell = None;
                let mut drop_target = None;
                let mut in_flow = false;
                // Placed in a row or column: (is it a column, its gap) for the insertion line.
                let mut slot_axis: Option<(bool, f64)> = None;
                // Only the selected layers moved, each by `d` (see `Interaction::moved_by`).
                let mut shifted = !dup;
                self.preview(|doc, sel| {
                    let mut moving = ids.clone();
                    if dup {
                        let mut copies = Vec::new();
                        for i in &ids {
                            let parent = doc.get(*i)?.parent.unwrap_or(page);
                            let idx = doc.index_in_parent(*i).map(|x| x + 1);
                            copies.push(crate::components::copy_in_file(doc, *i, parent, idx)?);
                        }
                        crate::library::count_inserted(doc, &copies);
                        // Connectors copied with what they join join the copies.
                        let map = crate::board::copy_map(doc, &ids, &copies);
                        crate::board::rejoin(doc, &crate::board::copied_items(doc, &map), &map, false);
                        moving = copies;
                        *sel = moving.clone();
                    }
                    let p = ev.p();
                    let home = moving.first().and_then(|i| doc.node(*i)).and_then(|n| n.parent);
                    // Absolute children of an auto layout frame stay in it, and so do the layers of
                    // groups, boolean groups and instances ([`crate::drop::keeps_layers`]); the
                    // rest may change parent.
                    let flow = moving.iter().all(|i| doc.node(*i).is_some_and(|n| !n.child_layout.absolute))
                        && !home.is_some_and(|h| crate::drop::keeps_layers(doc, h));
                    let target = if flow { crate::drop::drag_parent(doc, page, p, &moving) } else { home.unwrap_or(page) };
                    let mode = doc.node(target).and_then(|n| n.layout.as_ref()).map(|l| l.mode);
                    if let (true, Some(mode)) = (flow, mode) {
                        // Into (or within) an auto layout frame: the layers take the slot (a grid: the
                        // cell) under the pointer, as the frame shows it now. A frame other than the
                        // one they come from is outlined too.
                        let cell = if mode == LayoutMode::Grid { crate::drop::grid_cell_at(doc, target, p) } else { None };
                        // Within the frame they came from, the slot follows the leading edge of the
                        // box they would fill (their combined start bounds moved by the drag), not
                        // the pointer. The preview starts from the document as the drag began.
                        let within = moving.iter().all(|m| doc.node(*m).is_some_and(|n| n.parent == Some(target)));
                        let idx = if mode == LayoutMode::Grid {
                            0
                        } else if let (true, Some(b)) = (within, doc.bounds_of(&moving)) {
                            let moved = Rect::new(b.x0 + raw.x, b.y0 + raw.y, b.x1 + raw.x, b.y1 + raw.y);
                            crate::drop::auto_layout_slot_within(doc, target, p, &moving, moved, Point::new(raw.x, raw.y)).map_or(0, |(i, _)| i)
                        } else {
                            crate::drop::auto_layout_slot(doc, target, p, &moving).map_or(0, |(i, _)| i)
                        };
                        for m in &moving {
                            if doc.get(*m)?.parent != Some(target) {
                                doc.reparent(*m, target, None)?;
                            }
                        }
                        if let (Some(first), Some((col, row, rect))) = (moving.first().copied(), cell) {
                            let cl = &mut doc.get_mut(first)?.child_layout;
                            cl.grid_column = Some(col);
                            cl.grid_row = Some(row);
                            drop_cell = Some(rect);
                        } else if mode != LayoutMode::Grid {
                            let mut kids: Vec<NodeId> = doc.get(target)?.children.iter().copied().filter(|c| !moving.contains(c)).collect();
                            for (k, m) in moving.iter().enumerate() {
                                kids.insert((idx + k).min(kids.len()), *m);
                            }
                            let gap = doc.node(target).and_then(|n| n.layout.as_ref()).map_or(0.0, |l| l.gap);
                            doc.get_mut(target)?.children = kids;
                            slot_axis = Some((mode == LayoutMode::Vertical, gap));
                        }
                        drop_target = (Some(target) != home).then_some(target);
                        in_flow = true;
                        return Ok(());
                    }
                    // A free move. Layers dragged out of an auto layout flow leave it (onto the page
                    // while dragging; the drop puts them in the frame under the pointer).
                    shifted &= only_shifted(doc, &moving);
                    for m in &moving {
                        let in_al = doc.node(*m).and_then(|n| n.parent).and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some());
                        if flow && in_al {
                            doc.reparent(*m, page, None)?;
                        }
                        move_world(doc, *m, d.x, d.y)?;
                        if doc.node(*m).is_some_and(crate::snap::always_on_pixels) {
                            shifted &= !crate::snap::settle_on_pixels(doc, *m)?;
                        }
                    }
                    drop_target = (flow && target != page).then_some(target);
                    Ok(())
                })?;
                if let Some(it) = self.ds_mut()?.interaction.as_mut() {
                    it.moved_by = (shifted && !in_flow).then_some((d.x, d.y));
                }
                let placed = if dup { self.selection() } else { ids };
                // Snapping guides mean nothing while the layout places the layers; with Alt held,
                // live distances to the neighbours replace them.
                self.tool.overlay.guides = if in_flow {
                    Vec::new()
                } else if ev.alt {
                    let doc = self.doc()?;
                    doc.bounds_of(&placed).map(|b| crate::distance::neighbours(doc, page, &placed, b)).unwrap_or_default()
                } else {
                    guides
                };
                // The preview already shows the layers in their slot: the line marks it there, in
                // the middle of the gap before them (clear of the selection box).
                self.tool.overlay.insert_line = slot_axis.and_then(|(vertical, gap)| {
                    let off = if gap.is_finite() { (gap / 2.0).clamp(3.0, 24.0) } else { 3.0 };
                    self.doc().ok()?.bounds_of(&placed).map(|b| crate::drop::line_before(b, vertical, off))
                });
                self.tool.overlay.drop_cell = drop_cell;
                self.tool.overlay.drop_target = drop_target;
                self.tool.overlay.stamp_target = self.dragged_stamp_target(&placed, page);
                self.tool.overlay.cursor = if self.refused(&placed, true) {
                    Cursor::NotAllowed
                } else if dup {
                    Cursor::Copy
                } else {
                    Cursor::Default
                };
                self.tool.gesture = g.clone();
                if let Gesture::Move { moved, .. } = &mut self.tool.gesture {
                    *moved = true;
                }
                return Ok(());
            }
            Gesture::Resize { start, r, scale_tool, .. } => {
                let res = self.resize_drag(ev, *start, r, *scale_tool);
                self.tool.gesture = Gesture::Resize { start: *start, r: r.clone(), scale_tool: *scale_tool, moved: true };
                if self.refused(&self.selection(), false) {
                    self.tool.overlay.cursor = Cursor::NotAllowed;
                }
                return res;
            }
            Gesture::Rotate { center, start_angle, base, ids } => {
                let mut a = ((ev.p() - *center).atan2() - start_angle).to_degrees();
                if ev.shift {
                    // The panel rotation is base - a; Shift lands it on a multiple of 15 degrees.
                    a = base - ((base - a) / 15.0).round() * 15.0;
                }
                let (c, ids) = (*center, ids.clone());
                self.preview(|doc, _| {
                    for i in &ids {
                        crate::cmd::arrange::rotate_world(doc, *i, c, -a)?;
                    }
                    Ok(())
                })?;
                // The cursor turns with the pointer about the centre.
                self.tool.overlay.cursor =
                    if self.refused(&ids, false) { Cursor::NotAllowed } else { Cursor::Rotate((ev.p() - c).atan2().to_degrees()) };
                Ok(())
            }
            Gesture::Draw { start, id, parent, targets } => {
                let (start, id, parent, targets) = (*start, *id, *parent, targets.clone());
                self.tool.gesture = g;
                return self.draw_drag(ev, start, id, parent, &targets);
            }
            Gesture::TextBox { id, start, line } => {
                let (id, start, line) = (*id, *start, *line);
                self.tool.gesture = g;
                return self.text_box_drag(ev, id, start, line);
            }
            Gesture::Arc { id, handle, arc } => {
                let (id, handle, arc) = (*id, *handle, *arc);
                self.tool.gesture = g;
                return self.arc_drag(ev, id, handle, arc);
            }
            Gesture::Shape { id, handle } => {
                let (id, handle) = (*id, *handle);
                let doc = self.doc()?.clone();
                if let Some(props) = crate::shape_handles::drag(&doc, id, handle, ev.p(), ev.alt) {
                    self.preview(|doc, _| crate::cmd::props::apply("node.set", doc, id, &props))?;
                }
                self.tool.overlay.shape = shape_hint(self.doc()?, id, handle, ev.zoom);
                self.tool.overlay.shape_hover = Some(handle);
                Ok(())
            }
            Gesture::Pan { sx, sy, cx, cy } => {
                let ds = self.ds_mut()?;
                let z = ds.view.zoom.max(1e-6);
                ds.view.cx = cx - (ev.sx - sx) / z;
                ds.view.cy = cy - (ev.sy - sy) / z;
                ds.revision += 1;
                Ok(())
            }
            Gesture::Pencil { points, pressure } => {
                let (mut pts, mut pressure) = (points.clone(), pressure.clone());
                pencil_add(&mut pts, &mut pressure, ev);
                self.tool.overlay.pen = pts.iter().map(|p| [p.x, p.y]).collect();
                self.tool.gesture = Gesture::Pencil { points: pts, pressure };
                if let Ok(ds) = self.ds_mut() {
                    ds.revision += 1;
                }
                return Ok(());
            }
            Gesture::TextSelect { id } => {
                let id = *id;
                let idx = self.text_index_at(id, ev.p())?;
                textedit::set_caret(self, idx, true)?;
                Ok(())
            }
            Gesture::Vector(_) | Gesture::ShapeBuild { .. } | Gesture::Origin => Ok(()),
            Gesture::Connect { from, start } => {
                let (from, start) = (*from, *start);
                self.connect_drag(ev, from, start);
                return Ok(());
            }
            Gesture::Erase { gone } => {
                self.tool.gesture = Gesture::Erase { gone: gone.clone() };
                return self.erase_at(ev);
            }
            Gesture::Reattach { id, from, start } => {
                let (id, from, start) = (*id, *from, *start);
                self.reattach_drag(ev, id, from, start);
                return Ok(());
            }
            Gesture::Reshape { id, portion, start } => {
                let (id, portion, start) = (*id, *portion, *start);
                self.reshape_drag(ev, id, portion, start);
                return Ok(());
            }
            Gesture::PenDrag { anchor } => {
                // A click's wobble pulls out no handle.
                let out = if within_slop(ev, *anchor) { Vec2::ZERO } else { ev.p() - *anchor };
                if let Some(pen) = self.tool.pen.as_mut()
                    && let Some(last) = pen.points.last_mut()
                {
                    last.2 = out;
                    last.1 = -out;
                }
                self.pen_update()?;
                Ok(())
            }
        };
        self.tool.gesture = g;
        res
    }

    /// An arc handle drag: the new arc, written through `node.set`'s keys.
    fn arc_drag(&mut self, ev: &PointerEvent, id: NodeId, handle: ArcHandle, arc: crate::arc_handles::Arc) -> Result<()> {
        let doc = self.doc()?.clone();
        let Some(next) = crate::arc_handles::drag(&doc, id, handle, arc, ev.p(), ev.shift, ev.zoom) else { return Ok(()) };
        let props = crate::arc_handles::props(next);
        self.preview(|doc, _| crate::cmd::props::apply("node.set", doc, id, &props))?;
        self.tool.gesture = Gesture::Arc { id, handle, arc: next };
        let at = crate::arc_handles::handle_at(self.doc()?, id, handle, next, ev.zoom);
        self.tool.overlay.arc = at.map(|at| ArcHint { handle, label: next.label(handle), at });
        self.tool.overlay.arc_hover = Some(handle);
        Ok(())
    }

    fn resize_drag(&mut self, ev: &PointerEvent, start: Point, r: &ResizeStart, scale_tool: bool) -> Result<()> {
        let page = self.page()?;
        let snap = self.ds().map(|d| d.view.snap_objects).unwrap_or(true) && !ev.cmd && !ev.ctrl;
        let ids: Vec<NodeId> = r.nodes.iter().map(|n| n.0).collect();
        let targets = if snap { Some(Targets::gather(self.doc()?, page, &ids)) } else { None };
        let mut p = ev.p();
        let mut guides = Vec::new();
        let _ = start;
        // In the handle box's own axes (one layer: its local space).
        let (frame, w0, h0) = r.frame;
        // The pixel grid: an upright box's dragged sides land on whole pixels (the Scale tool
        // scales by the factor as dragged). Snapping to layers comes after, so it wins on the axes
        // it snaps.
        // Frames, sections and components land on whole pixels whatever the setting, also where a
        // layer they snap to doesn't.
        let boards = {
            let doc = self.doc()?;
            !scale_tool && crate::guides::axis_aligned(frame) && ids.iter().all(|i| doc.node(*i).is_some_and(crate::snap::always_on_pixels))
        };
        if boards || !scale_tool && self.ds().map(|d| d.view.snap_pixel).unwrap_or(true) && crate::guides::axis_aligned(frame) {
            p = Point::new(p.x.round(), p.y.round());
        }
        if let Some(t) = &targets {
            let (dx, dy, g) = t.snap_point(p.x, p.y, SNAP_PX / ev.zoom.max(1e-6));
            p.x += dx;
            p.y += dy;
            guides = g;
        }
        if boards {
            p = Point::new(p.x.round(), p.y.round());
        }
        let q = frame.inverse() * p;
        let is_line = r.single && ids.first().and_then(|i| self.doc().ok()?.node(*i).map(|n| matches!(n.kind, NodeKind::Line))).unwrap_or(false);
        if scale_tool {
            return self.scale_drag(ev, r, q, guides);
        }
        if is_line {
            // Line end handles: move the end point freely (length + angle).
            let (a, b) = match r.handle {
                Handle::Corner(0) => (p, frame * Point::new(w0, 0.0)),
                _ => (frame * Point::new(0.0, 0.0), p),
            };
            let id = ids.first().copied().unwrap_or(NodeId(0));
            self.preview(|doc, _| {
                let mut v = b - a;
                if ev.shift {
                    let ang = (v.atan2() / (std::f64::consts::PI / 4.0)).round() * std::f64::consts::PI / 4.0;
                    v = Vec2::from_angle(ang) * v.hypot();
                }
                let len = v.hypot();
                let world = Affine::translate(a.to_vec2()) * Affine::rotate(v.atan2());
                doc.set_world_transform(id, world)?;
                figcraft_layout::set_size(doc, id, len, 0.0);
                Ok(())
            })?;
            self.tool.overlay.guides = guides;
            return Ok(());
        }
        let image_box = ids.first().filter(|_| ev.cmd && r.single).and_then(|i| self.image_box(*i));
        // Constrain proportions (Shift, or the node's lock; cropping an image ignores the lock).
        let locked =
            r.single && image_box.is_none() && ids.first().and_then(|i| self.doc().ok()?.node(*i).map(|n| n.constrain_proportions)).unwrap_or(false);
        let keep = ev.shift || locked;
        let resized = resized_box(r.handle, q, w0, h0, keep, ev.alt);
        let (x0, y0, x1, y1) = match image_box {
            Some(image) => within_the_image(resized, image, (w0, h0), keep),
            None => resized,
        };
        let (flip_x, flip_y) = (x1 < x0, y1 < y0);
        let (nx0, ny0) = (x0.min(x1), y0.min(y1));
        let (nw, nh) = ((x1 - x0).abs(), (y1 - y0).abs());
        let nodes = r.nodes.clone();
        let single = r.single;
        let handle = r.handle;
        // What's in a section stays where it is on the page, and so does what's in a frame resized
        // with Cmd/Ctrl held (its constraints are ignored).
        let keeps = |doc: &Document, id: NodeId| {
            doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Section { .. }) || (ev.cmd && single && n.kind.is_frame_like()))
        };
        let dragged_box = |world: Affine, (ox, oy): (f64, f64)| {
            let mut t = world * Affine::translate((ox, oy));
            if flip_x {
                t *= Affine::new([-1.0, 0.0, 0.0, 1.0, nw, 0.0]);
            }
            if flip_y {
                t *= Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, nh]);
            }
            t
        };
        self.preview(|doc, _| {
            if single {
                let Some((id, world, _, _)) = nodes.first() else { return Ok(()) };
                let kept = if keeps(doc, *id) { crate::cmd::props::children_worlds(doc, *id) } else { Vec::new() };
                let t = dragged_box(*world, (nx0, ny0));
                // Text reflow and auto layout can recompute this box after the preview (a size bound
                // to a variable lets go of it: the resize is an explicit edit). A text keeps its
                // fills; any other layer's image is cropped again for the box layout leaves
                // (`crop_to_the_laid_out_box`).
                let n = doc.get(*id)?;
                // Crop the layer's image fills without moving their pixels on the page.
                // Work from the interaction snapshot, just like the geometry on every move.
                if ev.cmd && !matches!(n.kind, NodeKind::Text(_)) {
                    let fills = cropped_image_fills(doc, n, *world, t, (nw, nh));
                    doc.get_mut(*id)?.fills = fills;
                }
                doc.set_world_transform(*id, t)?;
                if kept.is_empty() {
                    crate::cmd::props::resize_node(doc, *id, nw.max(0.01), nh.max(0.01))?;
                } else {
                    figcraft_layout::set_size(doc, *id, nw.max(0.01), nh.max(0.01));
                    crate::cmd::props::restore_worlds(doc, &kept)?;
                }
                let n = doc.get_mut(*id)?;
                if matches!(handle, Handle::Corner(_) | Handle::Edge(1) | Handle::Edge(3)) {
                    n.child_layout.sizing_h = Sizing::Fixed;
                }
                if matches!(handle, Handle::Corner(_) | Handle::Edge(0) | Handle::Edge(2)) {
                    n.child_layout.sizing_v = Sizing::Fixed;
                }
                if let NodeKind::Text(t) = &mut n.kind {
                    match handle {
                        Handle::Edge(1) | Handle::Edge(3) if t.auto_resize == TextAutoResize::Truncate => {}
                        Handle::Edge(1) | Handle::Edge(3) => t.auto_resize = TextAutoResize::Height,
                        _ => t.fix_size(true, true),
                    }
                }
                return Ok(());
            }
            // Several layers: map the old box onto the new one, in the box's axes. Each layer's
            // centre follows the box (mirrored when it flips); each keeps its rotation and scales
            // along its own axes (see `axis_factors`), turning over with the box.
            let (sx, sy) = (if w0 > 1e-9 { (x1 - x0) / w0 } else { 1.0 }, if h0 > 1e-9 { (y1 - y0) / h0 } else { 1.0 });
            let mirror = Affine::new([sx.signum(), 0.0, 0.0, sy.signum(), 0.0, 0.0]);
            let inv = frame.inverse();
            for (id, world, w, h) in &nodes {
                let local = inv * *world;
                let center = local * Point::new(w / 2.0, h / 2.0);
                let (kw, kh) = figcraft_layout::axis_factors(local, sx, sy);
                let (w2, h2) = (w * kw, h * kh);
                let lin = (mirror * local).with_translation(Vec2::ZERO);
                let origin = Point::new(x0 + center.x * sx, y0 + center.y * sy) - lin * Point::new(w2 / 2.0, h2 / 2.0);
                let kept = if keeps(doc, *id) { crate::cmd::props::children_worlds(doc, *id) } else { Vec::new() };
                let (t, w2, h2) = if boards {
                    on_whole_pixels(doc, *id, frame * lin.with_translation(origin), w2, h2)
                } else {
                    (frame * lin.with_translation(origin), w2, h2)
                };
                doc.set_world_transform(*id, t)?;
                crate::cmd::props::resize_node(doc, *id, w2, h2)?;
                if boards {
                    crate::snap::settle_on_pixels(doc, *id)?;
                }
                crate::cmd::props::restore_worlds(doc, &kept)?;
            }
            Ok(())
        })?;
        if single
            && ev.cmd
            && let Some((id, world, _, _)) = nodes.first()
        {
            let laid_out_box = |(fw, fh): (f64, f64)| {
                let origin = match image_box {
                    Some((ix0, iy0, ix1, iy1)) => {
                        (slid_onto_the_image(nx0, fw, flip_x, (ix0, ix1), w0), slid_onto_the_image(ny0, fh, flip_y, (iy0, iy1), h0))
                    }
                    None => (nx0, ny0),
                };
                dragged_box(*world, origin)
            };
            self.crop_to_the_laid_out_box(*id, *world, laid_out_box, (nw, nh))?;
        }
        self.tool.overlay.guides = guides;
        Ok(())
    }

    /// The box `(x0, y0, x1, y1)` of the first visible, untiled image fill of `id` in the layer's
    /// own axes, as it was when the gesture began; `None` for a text or a layer without one.
    fn image_box(&self, id: NodeId) -> Option<(f64, f64, f64, f64)> {
        let snapshot = self.ds().ok()?.interaction.as_ref()?.doc.clone();
        let n = snapshot.node(id)?;
        if matches!(n.kind, NodeKind::Text(_)) {
            return None;
        }
        n.fills.iter().filter(|p| p.visible).find_map(|p| match &p.kind {
            PaintKind::Image(img) if img.scale_mode != ImageScaleMode::Tile => {
                let data = snapshot.images.get(&img.image)?;
                let crop = img.effective_crop(f64::from(data.width), f64::from(data.height), n.width, n.height);
                let (x0, y0, x1, y1) = figcraft_doc::color::ImagePaint::crop_rect(crop);
                Some((x0 * n.width, y0 * n.height, x1 * n.width, y1 * n.height))
            }
            _ => None,
        })
    }

    /// After the preview's layout: when the layer didn't keep the dragged size `(nw, nh)` (a
    /// minimum size, a Hug frame), crop its image fills again from the snapshot for the size it
    /// has, the box placed by `laid_out_box` (given that size) in the layer's starting axes.
    fn crop_to_the_laid_out_box(
        &mut self,
        id: NodeId,
        world: Affine,
        laid_out_box: impl Fn((f64, f64)) -> Affine,
        (nw, nh): (f64, f64),
    ) -> Result<()> {
        let ds = self.ds_mut()?;
        let Some(snapshot) = ds.interaction.as_ref().map(|it| it.doc.clone()) else { return Ok(()) };
        let Some(before) = snapshot.node(id).filter(|n| !matches!(n.kind, NodeKind::Text(_))) else { return Ok(()) };
        let Some((fw, fh)) = ds.doc.node(id).map(|n| (n.width, n.height)) else { return Ok(()) };
        if (fw - nw.max(0.01)).abs() < 1e-9 && (fh - nh.max(0.01)).abs() < 1e-9 {
            return Ok(());
        }
        let fills = cropped_image_fills(&snapshot, before, world, laid_out_box((fw, fh)), (fw, fh));
        std::sync::Arc::make_mut(&mut ds.doc).get_mut(id)?.fills = fills;
        ds.revision += 1;
        Ok(())
    }

    /// Scale tool handle drag (`q`: the snapped pointer in the handle box's axes): everything
    /// scales by one factor about the anchor point (Alt: the centre), strokes, effects, radii and
    /// text included, children by the same factor whatever their constraints.
    fn scale_drag(&mut self, ev: &PointerEvent, r: &ResizeStart, q: Point, guides: Vec<Guide>) -> Result<()> {
        let (frame, w0, h0) = r.frame;
        let Some((k, pivot)) = scale_factor(r.handle, q, w0, h0, ev.alt) else { return Ok(()) };
        let nodes = r.nodes.clone();
        let inv = frame.inverse();
        // A layer's place in the box scales about the pivot; a negative factor turns it over.
        let about = frame * Affine::translate(pivot.to_vec2() * (1.0 - k));
        let sign = k.signum();
        // A factor an instance can't follow (its scale factor would leave 0.001 to 1000) is not
        // applied: the drag keeps the last frame that fit.
        let mut refused = false;
        let shown = self.preview(|doc, _| {
            if !nodes.iter().all(|(id, ..)| crate::cmd::arrange::scale_fits(doc, *id, k.abs())) {
                refused = true;
                return Err(crate::error::bad("transform.scale", "past the scale range"));
            }
            for (id, world, _, _) in &nodes {
                let c = (inv * *world).as_coeffs();
                doc.set_world_transform(*id, about * Affine::new([sign * c[0], sign * c[1], sign * c[2], sign * c[3], k * c[4], k * c[5]]))?;
                crate::cmd::arrange::scale_node(doc, *id, k.abs())?;
            }
            Ok(())
        });
        if refused {
            return Ok(());
        }
        shown?;
        let p = frame * pivot;
        self.tool.overlay.scale = Some(ScaleHint { factor: k, pivot: [p.x, p.y] });
        self.tool.overlay.guides = guides;
        Ok(())
    }
}

/// `world` and the size `w` x `h` a layer of a resized selection takes, moved so its box in its
/// parent has its edges on whole pixels, each on the nearest one and the box at least a pixel
/// wide: a frame resized with others lands on whole pixels as one resized alone does. A layer
/// turned or scaled in its parent keeps them.
fn on_whole_pixels(doc: &Document, id: NodeId, world: Affine, w: f64, h: f64) -> (Affine, f64, f64) {
    let pw = doc.node(id).and_then(|n| n.parent).map_or(Affine::IDENTITY, |p| doc.world_transform(p));
    let local = pw.inverse() * world;
    let [a, b, c, d, e, f] = local.as_coeffs();
    if !crate::guides::axis_aligned(local) || (a.abs() - 1.0).abs() > 1e-9 || (d.abs() - 1.0).abs() > 1e-9 {
        return (world, w, h);
    }
    let edges = |at: f64, flip: bool, len: f64| if flip { (at - len, at) } else { (at, at + len) };
    let ((x0, x1), (y0, y1)) = (edges(e, a < 0.0, w), edges(f, d < 0.0, h));
    let (x0, y0) = (x0.round(), y0.round());
    let (w, h) = ((x1.round() - x0).max(1.0), (y1.round() - y0).max(1.0));
    let (e, f) = (if a < 0.0 { x0 + w } else { x0 }, if d < 0.0 { y0 + h } else { y0 });
    (pw * Affine::new([a, b, c, d, e, f]), w, h)
}

/// Moving `moving` by the same offset moves nothing else and changes nothing about them but where
/// they are: none is inside another (it would move twice) or placed by its parent's auto layout
/// (the layout would put it back, or close the gap it leaves).
fn only_shifted(doc: &Document, moving: &[NodeId]) -> bool {
    let set: std::collections::HashSet<NodeId> = moving.iter().copied().collect();
    moving.iter().all(|m| {
        let Some(n) = doc.node(*m) else { return false };
        let in_flow = n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some()) && !n.child_layout.absolute;
        let mut up = n.parent;
        let mut nested = false;
        while let Some(p) = up {
            nested |= set.contains(&p);
            up = doc.node(p).and_then(|x| x.parent);
        }
        !in_flow && !nested
    })
}

/// `before`'s fills (from the snapshot `doc`) with each image cropped to the box `dragged` (the
/// layer's new transform, `nw` × `nh`), its pixels left where they were on the page (`world`:
/// the layer's transform before the drag).
fn cropped_image_fills(
    doc: &Document,
    before: &figcraft_doc::Node,
    world: Affine,
    dragged: Affine,
    (nw, nh): (f64, f64),
) -> Vec<figcraft_doc::color::Paint> {
    let (w, h) = (before.width.max(1e-6), before.height.max(1e-6));
    let to_before = Affine::scale_non_uniform(1.0 / w, 1.0 / h) * world.inverse() * dragged * Affine::scale_non_uniform(nw.max(0.01), nh.max(0.01));
    let mut fills = before.fills.clone();
    for paint in &mut fills {
        let PaintKind::Image(img) = &mut paint.kind else { continue };
        // A repeating image cannot be represented by a single Crop placement.
        if img.scale_mode == ImageScaleMode::Tile {
            continue;
        }
        let Some(data) = doc.images.get(&img.image) else { continue };
        let (iw, ih) = (f64::from(data.width), f64::from(data.height));
        // A degenerate (zero-scale) layer has no inverse: its image keeps its placement.
        let crop = (Affine::new(img.placement_crop(iw, ih, w, h)) * to_before).as_coeffs();
        if !crop.iter().all(|c| c.is_finite()) {
            continue;
        }
        img.crop = Some(crop);
        img.scale_mode = ImageScaleMode::Crop;
    }
    fills
}

/// The extent `lo..hi` an image of extent `image` lets a crop reach on one axis of a layer that
/// started `len` long: the image's, or the layer's where that reaches further (a Fit image is
/// smaller than its layer).
fn crop_reach(image: (f64, f64), len: f64) -> (f64, f64) {
    (image.0.min(image.1).min(0.0), image.0.max(image.1).max(len))
}

/// The dragged box `(x0, y0, x1, y1)` drawn back towards the starting box `(0, 0, w0, h0)` until
/// its sides stop at the image's box `image` (see [`crop_reach`]). Both ends of an axis go back by
/// the same fraction, so an Alt resize stays centred, and with `keep` (Shift) both axes do, so the
/// ratio holds. A flipped axis stays as dragged.
fn within_the_image((x0, y0, x1, y1): (f64, f64, f64, f64), image: (f64, f64, f64, f64), (w0, h0): (f64, f64), keep: bool) -> (f64, f64, f64, f64) {
    let fraction = |a: f64, b: f64, image: (f64, f64), len: f64| {
        if b < a {
            return 1.0;
        }
        let (lo, hi) = crop_reach(image, len);
        let mut t: f64 = 1.0;
        if a < lo {
            t = t.min(lo / a);
        }
        if b > hi {
            t = t.min((hi - len) / (b - len));
        }
        if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 }
    };
    let (tx, ty) = (fraction(x0, x1, (image.0, image.2), w0), fraction(y0, y1, (image.1, image.3), h0));
    let (tx, ty) = if keep { (tx.min(ty), tx.min(ty)) } else { (tx, ty) };
    (x0 * tx, y0 * ty, w0 + (x1 - w0) * tx, h0 + (y1 - h0) * ty)
}

/// Where a crop box `len` long, placed at `start` on one axis, starts once it is slid back onto
/// the image (see [`crop_reach`]): layout can make a layer longer than it was dragged (a minimum
/// size), and the crop then moves along the image rather than past its edge. A flipped axis, or
/// a box longer than the reach, stays where it is.
fn slid_onto_the_image(start: f64, len: f64, flipped: bool, image: (f64, f64), len0: f64) -> f64 {
    let (lo, hi) = crop_reach(image, len0);
    if flipped || len.is_nan() || len > hi - lo {
        return start;
    }
    start.max(lo).min(hi - len)
}
