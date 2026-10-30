use super::derived::{ends_now, place_route, route};
use super::edit::set_label;
use super::items::add;
use super::{
    BoardItem, Connector, Document, End, LINE, LINE_WEIGHT, NodeId, NodeKind, Paint, Point, Result, Session, Side, StrokeCap, StrokeJoin, Value,
    VectorNetwork, bad, cap_of, get, id_of, json, num, req_targets, require, routing_of, string,
};

/// One connector end from the params: a layer on this page, or a point.
fn end_param(cmd: &str, s: &Session, p: &Value, key: &str, side_key: &str) -> Result<End> {
    let side = match string(p, side_key) {
        Some(v) => Side::from_id(&v).ok_or_else(|| bad(cmd, format!("`{side_key}` is auto, top, right, bottom or left")))?,
        None => Side::Auto,
    };
    match get(p, key) {
        Some(Value::Object(o)) if o.contains_key("x") => {
            let (x, y) = (num(&Value::Object(o.clone()), "x"), num(&Value::Object(o.clone()), "y"));
            match (x, y) {
                (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Ok(End::Point { x, y }),
                _ => Err(bad(cmd, format!("`{key}` needs finite x and y"))),
            }
        }
        Some(v) => {
            let node = id_of(v).ok_or_else(|| bad(cmd, format!("`{key}` is a layer id or {{x, y}}")))?;
            let doc = s.doc()?;
            let n = doc.get(node)?;
            if n.is_page() || doc.page_of(node) != Some(s.page()?) {
                return Err(bad(cmd, format!("`{key}` must be a layer on this page")));
            }
            if matches!(n.board, Some(BoardItem::Connector(_))) {
                return Err(bad(cmd, format!("`{key}` can't be another connector")));
            }
            Ok(End::Node { node, side })
        }
        None => Err(bad(cmd, format!("`{key}` is required"))),
    }
}

pub(super) fn connector_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.connector";
    require(s, cmd)?;
    let from = end_param(cmd, s, p, "from", "fromSide")?;
    let to = end_param(cmd, s, p, "to", "toSide")?;
    if from.node().is_some() && from.node() == to.node() {
        return Err(bad(cmd, "connect two different layers"));
    }
    let routing = match string(p, "style") {
        Some(v) => routing_of(cmd, &v)?,
        None => s.tool.board.routing,
    };
    let start = cap_of(cmd, p, "startCap")?.unwrap_or(CAPS.0);
    let end = cap_of(cmd, p, "endCap")?.unwrap_or(CAPS.1);
    let page = s.page()?;
    let doc = s.doc_mut()?;
    let id = make(doc, page, Connector { from, to, routing, path: None }, (start, end))?;
    let mut out = json!({"id": id});
    if let Some(text) = string(p, "label").filter(|t| !t.trim().is_empty()) {
        out["label"] = json!(set_label(doc, id, &text)?);
    }
    s.set_selection(vec![id]);
    Ok(out)
}

/// A new connector's (start, end) caps.
pub(super) const CAPS: (StrokeCap, StrokeCap) = (StrokeCap::None, StrokeCap::TriangleArrow);

/// Connector `c` added to `page` with its line, `caps` (start, end) and route: what
/// `board.connector` makes, and the connector tool shows while it is dragged.
pub(super) fn make(doc: &mut Document, page: NodeId, c: Connector, caps: (StrokeCap, StrokeCap)) -> Result<NodeId> {
    let cmd = "board.connector";
    let pts = route(doc, &c).ok_or_else(|| bad(cmd, "the ends can't be placed"))?;
    if pts.windows(2).all(|w| w.first() == w.get(1)) {
        return Err(bad(cmd, "the ends are in the same place"));
    }
    let id = add(doc, page, NodeKind::Vector { network: VectorNetwork::default() }, "Connector", (0.0, 0.0), (0.0, 0.0), |n| {
        n.fills.clear();
        n.strokes = vec![Paint::solid(LINE)];
        n.stroke.weight = LINE_WEIGHT;
        n.stroke.join = StrokeJoin::Round;
        (n.stroke.cap_start, n.stroke.cap_end) = caps;
        n.board = Some(BoardItem::Connector(c.clone()));
    })?;
    place_route(doc, id, &pts, c.routing);
    Ok(id)
}

/// The connector `id` names: a connector, or the label on one.
pub(super) fn connector_of(cmd: &str, doc: &Document, id: NodeId) -> Result<(NodeId, Connector)> {
    match doc.get(id)?.board.clone() {
        Some(BoardItem::Connector(c)) => Ok((id, c)),
        Some(BoardItem::Label { connector }) => match doc.get(connector)?.board.clone() {
            Some(BoardItem::Connector(c)) => Ok((connector, c)),
            _ => Err(bad(cmd, "that isn't a connector")),
        },
        _ => Err(bad(cmd, "that isn't a connector")),
    }
}

pub(super) fn reattach(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.reattach";
    require(s, cmd)?;
    let id = req_targets(cmd, s, p)?.first().copied().ok_or_else(|| bad(cmd, "select a connector"))?;
    let (id, mut c) = connector_of(cmd, s.doc()?, id)?;
    let from = match string(p, "end").as_deref() {
        Some("from") => true,
        Some("to") => false,
        _ => return Err(bad(cmd, "`end` is from or to")),
    };
    let end = end_param(cmd, s, p, "target", "side")?;
    let other = if from { c.to } else { c.from };
    if end.node().is_some() && end.node() == other.node() {
        return Err(bad(cmd, "connect two different layers"));
    }
    *(if from { &mut c.from } else { &mut c.to }) = end;
    // A path shaped by hand was for where the end was.
    c.path = None;
    let doc = s.doc_mut()?;
    let pts = route(doc, &c).ok_or_else(|| bad(cmd, "the ends can't be placed"))?;
    if pts.windows(2).all(|w| w.first() == w.get(1)) {
        return Err(bad(cmd, "the ends are in the same place"));
    }
    let routing = c.routing;
    doc.get_mut(id)?.board = Some(BoardItem::Connector(c));
    place_route(doc, id, &pts, routing);
    Ok(json!({"id": id}))
}

/// A connector end as `board.connector` and `board.reattach` take it.
pub(super) fn end_json(e: End) -> Value {
    match e {
        End::Node { node, .. } => json!(node),
        End::Point { x, y } => json!({"x": x, "y": y}),
    }
}

/// The side a connector end names, as the commands take it (none for a free point).
pub(super) fn side_json(e: End) -> Value {
    match e {
        End::Node { side, .. } => json!(side),
        End::Point { .. } => Value::Null,
    }
}

/// The end of connector `id` that stays while its other one (`from`: its start) is dragged.
pub(super) fn other_end(doc: &Document, id: NodeId, from: bool) -> Option<End> {
    match &doc.node(id)?.board {
        Some(BoardItem::Connector(c)) => Some(if from { c.to } else { c.from }),
        _ => None,
    }
}

/// The selected connector (one, alone and unlocked) and its two ends on the page as drawn.
pub fn selected_connector_ends(doc: &Document, sel: &[NodeId]) -> Option<(NodeId, [Point; 2])> {
    let [id] = sel else { return None };
    if !matches!(doc.node(*id)?.board, Some(BoardItem::Connector(_))) || crate::tools::locked_in(doc, *id) {
        return None;
    }
    let (a, b) = ends_now(doc, *id)?;
    Some((*id, [a, b]))
}

/// How near (screen pixels) a press must be to a selected connector's end to drag it.
pub const END_HANDLE_PX: f64 = 8.0;

/// The selected connector's end under `p` at `zoom`, nearest first (`true`: its start).
pub(crate) fn end_handle_at(doc: &Document, sel: &[NodeId], p: Point, zoom: f64) -> Option<(NodeId, bool)> {
    let (id, [a, b]) = selected_connector_ends(doc, sel)?;
    let (da, db) = ((a - p).hypot(), (b - p).hypot());
    (da.min(db) * zoom.max(1e-6) <= END_HANDLE_PX).then_some((id, da <= db))
}
