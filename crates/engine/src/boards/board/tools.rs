use super::connectors::{CAPS, at_json, end_json, make, other_end, side_json};
use super::derived::{ends_now, route};
use super::edit::label_of;
use super::items::edit_text;
use super::{
    BoardItem, Connector, Document, ERASER_PX, End, EngineError, Gesture, NodeId, Point, PointerEvent, Result, SHAPE, STICKY, Session, Side, ToolId,
    id_of, is_board, json,
};

/// How near (screen pixels) the pointer must be to the middle of an object's side for a connector
/// end to attach there, however small the object is on screen.
const ATTACH_PX: f64 = 10.0;

/// Point `p` as fractions of box `b` (`None`: a box with no area).
fn part_at(b: figcraft_geom::Rect, p: Point) -> Option<[f64; 2]> {
    (b.width() > 0.0 && b.height() > 0.0).then(|| [((p.x - b.x0) / b.width()).clamp(0.0, 1.0), ((p.y - b.y0) / b.height()).clamp(0.0, 1.0)])
}
/// The sides an object takes a connector end at (their middles are its attach points).
const SIDES: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];

/// The side of box `b` a connector end at `p` chooses (#1708): the one `p` is towards anywhere in
/// the box's outer half, whatever its proportions, so a whole edge picks its side. In the middle
/// of the box none is chosen ([`Side::Auto`]: the side facing the other end).
fn side_at(b: figcraft_geom::Rect, p: Point) -> Side {
    let c = b.center();
    let (x, y) = ((p.x - c.x) / (b.width() / 2.0).max(1e-6), (p.y - c.y) / (b.height() / 2.0).max(1e-6));
    match (x.abs().max(y.abs()) < 0.5, x.abs() >= y.abs()) {
        (true, _) => Side::Auto,
        (false, true) if x > 0.0 => Side::Right,
        (false, true) => Side::Left,
        (false, false) if y > 0.0 => Side::Bottom,
        (false, false) => Side::Top,
    }
}

// ---------------------------------------------------------------------------------------------
// The board tools.

impl Session {
    /// A press with a board tool: place a sticky, shape or stamp, start a connector or a stroke,
    /// or start erasing.
    pub(crate) fn board_down(&mut self, ev: &PointerEvent) -> Result<()> {
        // A board tool kept while a design file came to the front: back to Move for this press.
        if !is_board(self.doc()?) {
            self.tool.tool = ToolId::Move;
            return self.pointer(ev);
        }
        // A sticky still being typed in ends first, so the next object is an undo step of its own.
        if self.ds()?.text_edit.is_some() {
            crate::textedit::end(self)?;
        }
        match self.tool.tool {
            ToolId::Sticky => {
                let half = STICKY / 2.0;
                let r = self.execute("board.sticky", &json!({"x": ev.x - half, "y": ev.y - half}))?;
                self.board_placed();
                if let Some(id) = r.get("id").and_then(id_of) {
                    edit_text(self, id)?;
                }
                Ok(())
            }
            ToolId::BoardShape => {
                let half = SHAPE / 2.0;
                self.execute("board.shape", &json!({"x": ev.x - half, "y": ev.y - half}))?;
                self.board_placed();
                Ok(())
            }
            ToolId::Stamp => {
                // Stamps go on one after another: the tool stays.
                self.execute("board.stamp", &json!({"x": ev.x, "y": ev.y}))?;
                Ok(())
            }
            ToolId::Marker | ToolId::Highlighter => {
                self.tool.gesture = Gesture::Pencil { points: vec![ev.p()], pressure: vec![ev.pressure] };
                Ok(())
            }
            ToolId::Connector => {
                let from = self.end_at(ev);
                self.tool.gesture = Gesture::Connect { from, start: ev.p() };
                // While it is dragged the connector shows as it will be made (a preview).
                self.begin_interaction("Connector")
            }
            ToolId::Eraser => {
                self.begin_interaction("Erase")?;
                self.tool.gesture = Gesture::Erase { gone: Vec::new() };
                self.erase_at(ev)
            }
            _ => Ok(()),
        }
    }

    /// After placing an object: back to the Move tool unless tools stay on.
    fn board_placed(&mut self) {
        if !self.prefs.keep_tool {
            self.set_tool(ToolId::Move);
        }
    }

    /// What a connector end at the pointer attaches to: the object there (at the side whose middle
    /// is within [`ATTACH_PX`], else the side [`side_at`] the pointer; with Cmd/Ctrl held, right
    /// there), or the point.
    fn end_at(&self, ev: &PointerEvent) -> End {
        let zoom = ev.zoom.max(1e-6);
        let hit = (|| {
            let ds = self.ds().ok()?;
            // Under the connector being dragged out (previewed in the document), not on it.
            let doc = ds.interaction.as_ref().map_or(&*ds.doc, |it| &*it.doc);
            let leaf = self.hit(doc, ds.page, ev.p(), 2.0 / zoom)?;
            let t = figcraft_doc::geometry::select_target(doc, leaf, &[], false);
            let n = doc.node(t)?;
            if matches!(n.board, Some(BoardItem::Connector(_)) | Some(BoardItem::Label { .. }) | Some(BoardItem::Ink)) {
                return None;
            }
            let b = doc.world_bounds(t);
            let far = |s: &Side| b.map(|b| (s.point(b) - ev.p()).hypot() * zoom).filter(|d| *d <= ATTACH_PX);
            let near = SIDES.iter().filter_map(|s| Some((*s, far(s)?))).min_by(|x, y| x.1.total_cmp(&y.1)).map(|(s, _)| s);
            let side = near.or(b.map(|b| side_at(b, ev.p()))).unwrap_or(Side::Auto);
            match b.and_then(|b| part_at(b, ev.p())).filter(|_| ev.cmd) {
                Some(at) => Some(End::Node { node: t, side: Side::Auto, at: Some(at) }),
                None => Some(End::Node { node: t, side, at: None }),
            }
        })();
        hit.unwrap_or(End::Point { x: ev.x, y: ev.y })
    }

    /// Where a connector dragged from `from` (pressed at `start`) to the pointer ends: none for a
    /// click, or back on the object it started from (nothing is made).
    fn drag_end(&self, ev: &PointerEvent, from: End, start: Point) -> Option<End> {
        if (ev.p() - start).hypot() * ev.zoom.max(1e-6) < 4.0 {
            return None;
        }
        let to = self.end_at(ev);
        (to.node().is_none() || to.node() != from.node()).then_some(to)
    }

    /// The object `end` is on (if any) outlined, with its attach points; the one the end is at
    /// (`at`, else its named side's) lit.
    fn show_attach(&mut self, end: Option<End>, at: Option<Point>) {
        let attach = match end {
            Some(End::Node { node, side, .. }) => self.doc().ok().and_then(|d| d.world_bounds(node)).map(|b| {
                let on = at.or_else(|| (side != Side::Auto).then(|| side.point(b)));
                SIDES.map(|s| s.point(b)).map(|p| ([p.x, p.y], on.is_some_and(|o| (o - p).hypot() < 1e-6))).to_vec()
            }),
            _ => None,
        };
        self.tool.overlay.attach = attach.unwrap_or_default();
        self.tool.overlay.drop_target = end.and_then(|e| e.node());
    }

    /// A stamp among the layers `moving` (being dragged): the object it would stick to where it is
    /// now, under its centre (as [`super::settle_stamps`] decides when it is let go).
    pub(crate) fn dragged_stamp_target(&self, moving: &[NodeId], page: NodeId) -> Option<NodeId> {
        let doc = self.doc().ok().filter(|d| is_board(d))?;
        let stamp = moving.iter().copied().find(|id| matches!(doc.node(*id).and_then(|n| n.board.as_ref()), Some(BoardItem::Stamp { .. })))?;
        super::stamp_target(doc, page, doc.world_bounds(stamp)?.center(), Some(stamp))
    }

    /// The pointer moving with the connector tool and no button down: the attach points under it.
    pub(crate) fn connector_hover(&mut self, ev: &PointerEvent) {
        let end = self.doc().is_ok_and(is_board).then(|| self.end_at(ev));
        self.show_attach(end, None);
    }

    /// The connector being dragged out, drawn as it will be made (a preview) to the attach point
    /// or free point under the pointer.
    pub(crate) fn connect_drag(&mut self, ev: &PointerEvent, from: End, start: Point) {
        self.tool.gesture = Gesture::Connect { from, start };
        let c = self.drag_end(ev, from, start).map(|to| Connector { from, to, routing: self.tool.board.routing, path: None });
        if let Ok(page) = self.page() {
            // One that can't be placed shows nothing, as none would be made.
            let _ = self.preview(|d, _| {
                if let Some(c) = &c {
                    let _ = make(d, page, c.clone(), CAPS);
                }
                Ok(())
            });
        }
        let at = c.as_ref().and_then(|c| route(self.doc().ok()?, c)).and_then(|pts| pts.last().copied());
        self.show_attach(c.map(|c| c.to), at);
        if let Ok(ds) = self.ds_mut() {
            ds.revision += 1;
        }
    }

    pub(crate) fn connect_up(&mut self, ev: &PointerEvent, from: End, start: Point) -> Result<()> {
        self.show_attach(None, None);
        // The preview goes: the command makes the connector, one undo step.
        self.cancel_interaction()?;
        let Some(to) = self.drag_end(ev, from, start) else { return Ok(()) };
        let style = self.tool.board.routing.id();
        let (from_at, to_at) = (at_json(from), at_json(to));
        let p = json!({"from": end_json(from), "fromSide": side_json(from), "fromAt": from_at, "to": end_json(to), "toSide": side_json(to), "toAt": to_at, "style": style});
        self.execute("board.connector", &p)?;
        self.board_placed();
        Ok(())
    }

    /// Dragging the selected connector `id`'s end (`from`: its start): a line from its other end
    /// to the pointer, and the object it would attach to outlined.
    pub(crate) fn reattach_drag(&mut self, ev: &PointerEvent, id: NodeId, from: bool, start: Point) {
        self.tool.gesture = Gesture::Reattach { id, from, start };
        let Ok(doc) = self.doc() else { return };
        let fixed = ends_now(doc, id).map(|(a, b)| if from { b } else { a });
        let other = other_end(doc, id, from).and_then(|e| e.node());
        self.tool.overlay.rubber = fixed.map(|f| [f.x, f.y, ev.x, ev.y]);
        let target = Some(self.end_at(ev)).filter(|e| e.node().is_none() || e.node() != other);
        self.show_attach(target, None);
        if let Ok(ds) = self.ds_mut() {
            ds.revision += 1;
        }
    }

    /// The end let go: attached to the object under the pointer, or free there (`board.reattach`).
    /// A click, or a drop on the object the other end is on, changes nothing.
    pub(crate) fn reattach_up(&mut self, ev: &PointerEvent, id: NodeId, from: bool, start: Point) -> Result<()> {
        self.tool.overlay.rubber = None;
        self.show_attach(None, None);
        if (ev.p() - start).hypot() * ev.zoom.max(1e-6) < 4.0 {
            return Ok(());
        }
        let target = self.end_at(ev);
        let other = other_end(self.doc()?, id, from).and_then(|e| e.node());
        if target.node().is_some() && target.node() == other {
            return Ok(());
        }
        let end = if from { "from" } else { "to" };
        self.execute("board.reattach", &json!({"id": id, "end": end, "target": end_json(target), "side": side_json(target), "at": at_json(target)}))?;
        Ok(())
    }

    /// The eraser at the pointer: the marker and highlighter strokes it touches go (until the
    /// gesture ends, which is one undo step).
    pub(crate) fn erase_at(&mut self, ev: &PointerEvent) -> Result<()> {
        let Gesture::Erase { mut gone } = std::mem::take(&mut self.tool.gesture) else { return Ok(()) };
        let tol = ERASER_PX / ev.zoom.max(1e-6);
        self.tool.overlay.brush = Some([ev.x, ev.y, tol]);
        let (doc, page) = (self.doc()?.clone(), self.page()?);
        if let Some(hit) =
            doc.hit_test(page, ev.p(), tol, None).filter(|h| doc.node(*h).is_some_and(|n| n.board == Some(BoardItem::Ink)) && !gone.contains(h))
        {
            gone.push(hit);
            let ids = gone.clone();
            self.preview(|d, sel| {
                for i in &ids {
                    let _ = d.remove(*i);
                }
                sel.retain(|s| !ids.contains(s));
                Ok(())
            })?;
        }
        self.tool.gesture = Gesture::Erase { gone };
        if let Ok(ds) = self.ds_mut() {
            ds.revision += 1;
        }
        Ok(())
    }

    pub(crate) fn erase_up(&mut self) -> Result<()> {
        self.tool.overlay.brush = None;
        self.commit_interaction()
    }

    /// A finished marker or highlighter stroke (page points).
    pub(crate) fn ink_finish(&mut self, path: &str) -> Result<()> {
        let high = self.tool.tool == ToolId::Highlighter;
        self.execute("board.ink", &json!({"path": path, "highlighter": high, "select": false}))?;
        // Strokes come one after another: the selection stays as it was.
        Ok(())
    }
}

impl Session {
    /// A double-click with the Move tool on a board object: type in its text (a connector gets a
    /// label to type in). True when it was one.
    pub(crate) fn board_double_click(&mut self, doc: &Document, leaf: NodeId, sel: &[NodeId]) -> Result<bool> {
        let whole = figcraft_doc::geometry::select_target(doc, leaf, sel, false);
        let editing = self.ds()?.text_edit.as_ref().is_some_and(|te| doc.is_ancestor(whole, te.id));
        let Some(item) = doc.node(whole).and_then(|n| n.board.clone()).filter(|_| !editing) else { return Ok(false) };
        self.tool.gesture = Gesture::None;
        self.cancel_interaction()?;
        match item {
            BoardItem::Connector(_) => {
                let label = match label_of(doc, whole) {
                    Some(l) => l,
                    None => {
                        let r = self.execute("board.label", &json!({"id": whole, "text": "Label"}))?;
                        r.get("label").and_then(id_of).ok_or_else(|| EngineError::Other("no label made".into()))?
                    }
                };
                edit_text(self, label)
            }
            BoardItem::Table => crate::board_table::edit_cell(self, doc, leaf),
            b if b.has_text() => edit_text(self, whole),
            _ => Ok(false),
        }
    }
}
