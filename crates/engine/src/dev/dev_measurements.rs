//! Measurements kept on the canvas (Dev Mode): a developer places one between two layers, or
//! across one side of a layer, and everyone who opens the file sees it. Each is stored on the layer
//! it starts from (`Node::measurements`: saved, synced, undoable) and drawn from the layers'
//! current bounds, so it follows them when they move or resize. One whose other layer was deleted
//! or moved to another page is neither drawn nor listed (undoing the deletion brings it back).

use figcraft_doc::{Document, Measurement, Node, NodeId, Side};
use figcraft_geom::{Point, Rect};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::distance::{between, measured};
use crate::params::{get, opt_id, opt_index};
use crate::snap::Guide;
use crate::{EngineError, Result, Session, bad, spec};

/// Most measurements starting from one layer.
pub const MAX_PER_LAYER: usize = 64;
/// Most measurements listed or drawn for one page.
pub const MAX_LISTED: usize = 2_000;

/// A measurement on a page: the layer it starts from, its index there, what it measures and the
/// lines it draws (page space).
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub from: NodeId,
    pub index: usize,
    pub measurement: Measurement,
    pub lines: Vec<Guide>,
}

impl Placed {
    /// Whether it starts or ends at layer `id`.
    pub fn touches(&self, id: NodeId) -> bool {
        self.from == id || self.measurement.to == Some(id)
    }

    /// The distances it shows, as on its labels.
    pub fn labels(&self) -> Vec<&str> {
        self.lines.iter().filter_map(|g| g.label.as_deref()).collect()
    }
}

/// The line along side `side` of box `b`, labelled with that side's length.
fn side_line(b: Rect, side: Side) -> Vec<Guide> {
    let (p, q) = match side {
        Side::Top => (Point::new(b.x0, b.y0), Point::new(b.x1, b.y0)),
        Side::Bottom => (Point::new(b.x0, b.y1), Point::new(b.x1, b.y1)),
        Side::Left => (Point::new(b.x0, b.y0), Point::new(b.x0, b.y1)),
        Side::Right => (Point::new(b.x1, b.y0), Point::new(b.x1, b.y1)),
        Side::Auto => return Vec::new(),
    };
    let mut out = Vec::new();
    measured(&mut out, p, q);
    out
}

/// The lines measurement `m` from layer `from` draws, in page space (none when a layer is gone).
pub fn lines(doc: &Document, from: NodeId, m: Measurement) -> Vec<Guide> {
    let Some(a) = doc.world_bounds(from) else { return Vec::new() };
    match (m.to, m.side) {
        (Some(to), _) => doc.world_bounds(to).map(|b| between(a, b)).unwrap_or_default(),
        (None, Some(side)) => side_line(a, side),
        (None, None) => Vec::new(),
    }
}

/// The measurements on `page` whose layers are both there, in layer order (bounded).
pub fn on_page(doc: &Document, page: NodeId) -> Vec<Placed> {
    let mut out = Vec::new();
    let mut stack = vec![page];
    let mut visited = 0usize;
    while let Some(id) = stack.pop() {
        visited += 1;
        if out.len() >= MAX_LISTED || visited > 200_000 {
            break;
        }
        let Some(n) = doc.node(id) else { continue };
        for (index, m) in n.measurements.iter().enumerate() {
            if m.to.is_some_and(|t| t == id || doc.page_of(t) != Some(page)) {
                continue;
            }
            out.push(Placed { from: id, index, measurement: *m, lines: lines(doc, id, *m) });
        }
        stack.extend(n.children.iter().rev().copied());
    }
    out
}

fn placed_json(doc: &Document, p: &Placed) -> Value {
    let name = |id: NodeId| doc.node(id).map(|n| n.name.clone()).unwrap_or_default();
    let mut j = json!({"from": p.from, "fromName": name(p.from), "index": p.index, "labels": p.labels(), "lines": p.lines});
    if let Some(t) = p.measurement.to {
        j["to"] = json!(t);
        j["toName"] = json!(name(t));
    }
    if let Some(s) = p.measurement.side {
        j["side"] = json!(s);
    }
    j
}

/// The layer a command starts from: `from`, else the first selected layer; never a page.
fn from_layer(cmd: &str, s: &Session, p: &Value) -> Result<NodeId> {
    let id = match opt_id(cmd, p, "from")? {
        Some(id) => id,
        None => s.selection().first().copied().ok_or_else(|| bad(cmd, "select a layer or pass `from`"))?,
    };
    let n = s.doc()?.node(id).ok_or(EngineError::NoNode(id))?;
    if n.is_page() {
        return Err(bad(cmd, "measure from a layer, not a page"));
    }
    Ok(id)
}

/// The `side` parameter: top, right, bottom or left.
fn side_param(cmd: &str, p: &Value) -> Result<Option<Side>> {
    match get(p, "side") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Side::from_id(v).filter(|s| *s != Side::Auto).map(Some).ok_or_else(|| bad(cmd, "side: top, right, bottom or left")),
        Some(_) => Err(bad(cmd, "side: top, right, bottom or left")),
    }
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "devMode.addMeasurement";
    let from = from_layer(cmd, s, p)?;
    let (to, side) = (opt_id(cmd, p, "to")?, side_param(cmd, p)?);
    let doc = s.doc()?;
    let n = doc.node(from).ok_or(EngineError::NoNode(from))?;
    let to = match (to, side) {
        (Some(_), Some(_)) => return Err(bad(cmd, "pass `to` or `side`, not both")),
        (Some(t), None) => Some(t),
        (None, Some(_)) => None,
        // Neither: to the layer around it.
        (None, None) => Some(
            n.parent
                .filter(|q| doc.node(*q).is_some_and(|q| !q.is_page()))
                .ok_or_else(|| bad(cmd, "pass `to` or `side` (the layer is at the top of its page)"))?,
        ),
    };
    if let Some(t) = to {
        let target = doc.node(t).ok_or(EngineError::NoNode(t))?;
        if t == from {
            return Err(bad(cmd, "measure to another layer, or across a `side` of this one"));
        }
        if target.is_page() || doc.page_of(t) != doc.page_of(from) {
            return Err(bad(cmd, "measure to a layer on the same page"));
        }
    }
    let m = Measurement { to, side };
    // The same two layers measured from the other end are the same measurement.
    let back = to.and_then(|t| doc.node(t)).is_some_and(|t| t.measurements.contains(&Measurement { to: Some(from), side: None }));
    if n.measurements.contains(&m) || back {
        return Err(bad(cmd, "that measurement is already there"));
    }
    if n.measurements.len() >= MAX_PER_LAYER {
        return Err(bad(cmd, format!("a layer starts at most {MAX_PER_LAYER} measurements")));
    }
    let drawn = lines(doc, from, m);
    let n = s.doc_mut()?.get_mut(from)?;
    n.measurements.push(m);
    Ok(json!({"from": from, "index": n.measurements.len() - 1, "lines": drawn}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "devMode.removeMeasurement";
    let from = from_layer(cmd, s, p)?;
    let index = opt_index(cmd, p, "index")?;
    let n = s.doc_mut()?.get_mut(from)?;
    let count = n.measurements.len();
    if count == 0 {
        return Err(bad(cmd, "no measurement starts from this layer"));
    }
    let removed = match index {
        Some(i) if i < count => {
            n.measurements.remove(i);
            1
        }
        Some(i) => return Err(bad(cmd, format!("no measurement {i} (the layer starts {count})"))),
        None => std::mem::take(&mut n.measurements).len(),
    };
    Ok(json!({"removed": removed}))
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "devMode.measurements";
    let doc = s.doc()?;
    let page = match opt_id(cmd, p, "page")? {
        Some(pg) if doc.node(pg).is_some_and(Node::is_page) => pg,
        Some(_) => return Err(bad(cmd, "`page` must be a page id")),
        None => s.page()?,
    };
    let only = opt_id(cmd, p, "id")?;
    Ok(Value::Array(on_page(doc, page).iter().filter(|m| only.is_none_or(|id| m.touches(id))).map(|m| placed_json(doc, m)).collect()))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "devMode.addMeasurement",
            "Add measurement",
            "{from? (default the first selected layer), to? (a layer on the same page), side? (top|right|bottom|left)}  keeps a measurement on the canvas: between `from` and `to`, across one side of `from`, or (neither) to the layer around it; Dev Mode draws it and it follows the layers → {from, index, lines}",
            true,
            add
        ),
        spec!(
            "devMode.removeMeasurement",
            "Remove measurement",
            "{from? (default the first selected layer), index? (omit: every measurement starting from the layer)}  → {removed}",
            true,
            remove
        ),
        spec!(
            "devMode.measurements",
            "List measurements",
            "{page? (default the current page), id? (only those from or to this layer)}  → [{from, fromName, index, to?, toName?, side?, labels, lines: [{x0, y0, x1, y1, label, dashed}]}] (page space)",
            false,
            list
        ),
    ]
}
