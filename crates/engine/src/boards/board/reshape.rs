//! A connector's path reshaped by hand: a selected connector's path handles (one for each run of
//! an elbow, one for a curve), dragged, and `board.reshape`, which a drag ends in.

use figcraft_doc::{attached_sides, path_handles, reshaped};

use super::connectors::{END_HANDLE_PX, connector_of, selected_connector_ends};
use super::derived::{place_route, route, route_ends};
use super::{
    BoardItem, Connector, Document, End, Gesture, NodeId, Point, PointerEvent, Result, Session, Side, Value, bad, json, req_num, req_targets, require,
};

/// How long a run must be on screen (pixels) to show a handle: shorter, it would sit on what's
/// next to it.
const MIN_RUN_PX: f64 = 28.0;

/// `end` with the side it leaves by named: a reshaped path holds only as long as its ends do.
fn pinned(end: End, side: Option<Side>) -> End {
    match (end, side) {
        (End::Node { node, .. }, Some(side)) => End::Node { node, side },
        _ => end,
    }
}

/// Connector `c` with `portion` of its path moved to `p` (`None`: it has no such portion).
fn moved(doc: &Document, c: &Connector, portion: usize, p: Point) -> Option<Connector> {
    let (from, to) = route_ends(doc, c)?;
    let path = reshaped(from, to, c.routing, c.path.as_ref(), portion, p)?;
    let (s0, s1) = attached_sides(from, to);
    Some(Connector { from: pinned(c.from, s0), to: pinned(c.to, s1), routing: c.routing, path: Some(path) })
}

/// Write `c` into connector `id` and run it along its new route.
fn place(doc: &mut Document, id: NodeId, c: Connector) -> Option<()> {
    let pts = route(doc, &c)?;
    let routing = c.routing;
    doc.node_mut(id)?.board = Some(BoardItem::Connector(c));
    place_route(doc, id, &pts, routing);
    Some(())
}

/// `board.reshape {id?, portion, x, y}`.
pub(super) fn reshape(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.reshape";
    require(s, cmd)?;
    let id = req_targets(cmd, s, p)?.first().copied().ok_or_else(|| bad(cmd, "select a connector"))?;
    let (id, c) = connector_of(cmd, s.doc()?, id)?;
    let portion = req_num(cmd, p, "portion")?;
    let (x, y) = (req_num(cmd, p, "x")?, req_num(cmd, p, "y")?);
    let refused = || bad(cmd, "that connector has no such portion to move there");
    if !(portion >= 0.0 && portion.fract() == 0.0 && portion < 1e6) {
        return Err(refused());
    }
    let doc = s.doc_mut()?;
    let c = moved(doc, &c, portion as usize, Point::new(x, y)).ok_or_else(refused)?;
    place(doc, id, c).ok_or_else(refused)?;
    Ok(json!({"id": id}))
}

/// The selected connector's path handles at `zoom`: the portion each moves and where it sits.
pub fn selected_connector_handles(doc: &Document, sel: &[NodeId], zoom: f64) -> Vec<(usize, Point)> {
    let handles = || {
        let (id, _) = selected_connector_ends(doc, sel)?;
        let (_, c) = connector_of("", doc, id).ok()?;
        let pts = route(doc, &c)?;
        let long = |i: usize| pts.windows(2).nth(i).is_some_and(|w| (w[1] - w[0]).hypot() * zoom >= MIN_RUN_PX);
        let curved = c.routing == figcraft_doc::Routing::Curved;
        Some(path_handles(&pts, c.routing).into_iter().filter(|(i, _)| curved || long(*i)).collect())
    };
    handles().unwrap_or_default()
}

/// The selected connector's path handle under `p` at `zoom`: the connector and the portion.
pub(crate) fn path_handle_at(doc: &Document, sel: &[NodeId], p: Point, zoom: f64) -> Option<(NodeId, usize)> {
    let [id] = sel else { return None };
    let dist = |h: &(usize, Point)| (h.1 - p).hypot();
    let near = |h: &(usize, Point)| dist(h) * zoom.max(1e-6) <= END_HANDLE_PX;
    let nearest = selected_connector_handles(doc, sel, zoom).into_iter().filter(near).min_by(|a, b| dist(a).total_cmp(&dist(b)));
    nearest.map(|(portion, _)| (*id, portion))
}

impl Session {
    /// A press on a selected connector's path handle.
    pub(crate) fn reshape_down(&mut self, id: NodeId, portion: usize, start: Point) -> Result<()> {
        // While it is dragged the connector shows as it will be (a preview).
        self.begin_interaction("Reshape connector")?;
        self.tool.gesture = Gesture::Reshape { id, portion, start };
        Ok(())
    }

    /// The handle dragged: the connector runs through the pointer, as it will once let go.
    pub(crate) fn reshape_drag(&mut self, ev: &PointerEvent, id: NodeId, portion: usize, start: Point) {
        self.tool.gesture = Gesture::Reshape { id, portion, start };
        // One that can't be reshaped there stays as it was.
        let _ = self.preview(|d, _| {
            let now = d.node(id).and_then(|n| if let Some(BoardItem::Connector(c)) = &n.board { moved(d, c, portion, ev.p()) } else { None });
            if let Some(c) = now {
                place(d, id, c);
            }
            Ok(())
        });
        if let Ok(ds) = self.ds_mut() {
            ds.revision += 1;
        }
    }

    /// The handle let go: `board.reshape`, one undo step. A click changes nothing.
    pub(crate) fn reshape_up(&mut self, ev: &PointerEvent, id: NodeId, portion: usize, start: Point) -> Result<()> {
        // The preview goes: the command reshapes the connector.
        self.cancel_interaction()?;
        if (ev.p() - start).hypot() * ev.zoom.max(1e-6) < 4.0 {
            return Ok(());
        }
        self.execute("board.reshape", &json!({"id": id, "portion": portion, "x": ev.x, "y": ev.y}))?;
        Ok(())
    }
}
