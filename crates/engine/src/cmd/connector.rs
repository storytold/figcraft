//! Connectors: lines whose ends attach to layers and follow them (the routing is
//! `figcraft_doc::connector`; [`crate::normalize`] re-routes them after every edit).

use figcraft_doc::{ConnectorEnd, ConnectorLineType, Document, Magnet, Node, NodeId, NodeKind};
use figcraft_geom::{Rect, Vec2};
use serde_json::{Value, json};

use super::CommandSpec;
use crate::params::*;
use crate::{Result, Session, bad, spec};

/// Farthest a free end may sit from the page origin.
const MAX_COORD: f64 = 1e7;

/// How an end is written in commands: `{node, magnet?}` attached, or `{x, y}` free.
pub const END_HELP: &str = "{node (layer id), magnet?: auto|top|right|bottom|left|center} attached, or {x, y} (page coordinates) free";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "connector.create",
            "Connector",
            "{start, end (each {node, magnet?: auto|top|right|bottom|left|center} or {x, y} in page coordinates), lineType?: straight|elbowed|curved (default elbowed), parent?, name?, props?} → {id}  a line whose attached ends follow their layers",
            true,
            create
        ),
        spec!(
            "connector.next",
            "Add connected shape",
            "{id? (default the selected shape), side?: top|right|bottom|left (default right)} → {id, connector}  a copy of the shape on that side, joined to it by a connector (Cmd/Ctrl+Return)",
            true,
            next
        ),
        spec!(
            "connector.detach",
            "Detach connector",
            "{ids?, end?: start|end|both (default both)} (the ends stay where they are, free)",
            true,
            detach
        ),
    ]
}

/// Parse one end for `cmd`: attached ends must name a layer on the page (not another connector).
pub fn parse_end(cmd: &str, doc: &Document, v: &Value) -> Result<ConnectorEnd> {
    if !v.is_object() {
        return Err(bad(cmd, format!("a connector end is {END_HELP}")));
    }
    let magnet = match string(v, "magnet") {
        Some(m) => Magnet::parse(&m).ok_or_else(|| bad(cmd, "`magnet` is auto|top|right|bottom|left|center"))?,
        None => Magnet::Auto,
    };
    if let Some(node) = id(v, "node").or_else(|| id(v, "id")) {
        let n = doc.get(node)?;
        if n.is_page() || matches!(n.kind, NodeKind::Connector { .. }) {
            return Err(bad(cmd, "connector ends attach to layers (not pages or other connectors)"));
        }
        let mut e = ConnectorEnd::attached(node, magnet);
        // Where it starts out: the layer's centre (routing moves it to the side).
        if let Some(r) = doc.world_bounds(node) {
            (e.x, e.y) = (r.center().x, r.center().y);
        }
        return Ok(e);
    }
    let (Some(x), Some(y)) = (num(v, "x"), num(v, "y")) else {
        return Err(bad(cmd, format!("a connector end is {END_HELP}")));
    };
    if x.abs() > MAX_COORD || y.abs() > MAX_COORD {
        return Err(bad(cmd, "connector end out of range"));
    }
    Ok(ConnectorEnd::free(x, y))
}

/// An end as commands and inspect write it.
pub fn end_json(e: &ConnectorEnd) -> Value {
    match e.node {
        Some(n) => json!({"node": n, "magnet": e.magnet, "x": e.x, "y": e.y}),
        None => json!({"x": e.x, "y": e.y}),
    }
}

pub fn line_type_of(cmd: &str, v: &Value) -> Result<ConnectorLineType> {
    v.as_str().and_then(ConnectorLineType::parse).ok_or_else(|| bad(cmd, "`lineType` is straight|elbowed|curved"))
}

pub fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "connector.create";
    let page = s.page()?;
    let parent = id(p, "parent").unwrap_or(page);
    let doc = s.doc()?;
    if !doc.get(parent)?.kind.is_container() {
        return Err(bad(cmd, format!("{parent} can't contain layers")));
    }
    let start = parse_end(cmd, doc, get(p, "start").unwrap_or(&Value::Null))?;
    let end = parse_end(cmd, doc, get(p, "end").unwrap_or(&Value::Null))?;
    let page_of = |e: &ConnectorEnd| e.node.and_then(|n| doc.page_of(n));
    if [page_of(&start), page_of(&end)].iter().flatten().any(|pg| Some(*pg) != doc.page_of(parent).or(Some(parent))) {
        return Err(bad(cmd, "connector ends attach to layers on the connector's page"));
    }
    if start.node.is_some() && start.node == end.node {
        return Err(bad(cmd, "a connector's ends attach to two different layers"));
    }
    let line = match get(p, "lineType") {
        Some(v) => line_type_of(cmd, v)?,
        None => ConnectorLineType::Elbowed,
    };
    let name = string(p, "name").unwrap_or_else(|| "Connector".to_string());
    let ds = s.ds_mut()?;
    let doc = ds.doc_mut();
    let nid = doc.alloc_id();
    let mut node = Node::new_connector(nid, name, start, end);
    if let NodeKind::Connector { line_type, .. } = &mut node.kind {
        *line_type = line;
    }
    let index = num(p, "index").map(|i| i.max(0.0) as usize);
    doc.insert(node, parent, index)?;
    // Route it now so `props` (x, y…) and callers see its real box.
    figcraft_doc::connector::reroute(doc, nid);
    if let Some(Value::Object(props)) = get(p, "props") {
        super::props::apply(cmd, doc, nid, props)?;
    }
    if boolean(p, "select").unwrap_or(true) {
        ds.selection = vec![nid];
    }
    Ok(json!({"id": nid}))
}

fn detach(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "connector.detach";
    let ids = req_targets(cmd, s, p)?;
    let which = string(p, "end").unwrap_or_else(|| "both".into());
    let (a, b) = match which.as_str() {
        "start" => (true, false),
        "end" => (false, true),
        "both" => (true, true),
        _ => return Err(bad(cmd, "`end` is start|end|both")),
    };
    let doc = s.ds_mut()?.doc_mut();
    let mut done: Vec<NodeId> = Vec::new();
    for i in ids {
        if let Some(n) = doc.node_mut(i)
            && let NodeKind::Connector { start, end, .. } = &mut n.kind
        {
            if a {
                start.node = None;
            }
            if b {
                end.node = None;
            }
            done.push(i);
        }
    }
    if done.is_empty() {
        return Err(bad(cmd, "nothing selected is a connector"));
    }
    Ok(json!({"ids": done}))
}

/// The space between a shape and the next one [`next`] adds beside it.
pub const NEXT_GAP: f64 = 80.0;

/// The outward direction of `side` (`None` for auto and centre).
fn side_dir(side: Magnet) -> Option<Vec2> {
    match side {
        Magnet::Top => Some(Vec2::new(0.0, -1.0)),
        Magnet::Right => Some(Vec2::new(1.0, 0.0)),
        Magnet::Bottom => Some(Vec2::new(0.0, 1.0)),
        Magnet::Left => Some(Vec2::new(-1.0, 0.0)),
        Magnet::Auto | Magnet::Center => None,
    }
}

/// Where [`next`] puts the copy of `id` added on `side`: how far it moves from `id`. Straight
/// across the gap, or, when something is already there, fanned out sideways to the first free
/// place.
pub fn next_offset(doc: &Document, id: NodeId, side: Magnet) -> Option<Vec2> {
    let d = side_dir(side)?;
    let b = doc.world_bounds(id)?;
    let n = doc.node(id)?;
    let parent = n.parent?;
    let along = if d.x != 0.0 { b.width() } else { b.height() } + NEXT_GAP;
    let across = if d.x != 0.0 { b.height() } else { b.width() } + NEXT_GAP / 2.0;
    let perp = Vec2::new(d.y.abs(), d.x.abs());
    // What's already beside it: the other layers in its parent (not connectors or shape texts).
    let others: Vec<Rect> = doc
        .node(parent)?
        .children
        .iter()
        .filter(|c| **c != id)
        .filter_map(|c| doc.node(*c))
        .filter(|c| c.visible && c.text_host.is_none() && !matches!(c.kind, NodeKind::Connector { .. }))
        .filter_map(|c| doc.world_bounds(c.id))
        .collect();
    let free = |off: Vec2| {
        let r = Rect::new(b.x0 + off.x, b.y0 + off.y, b.x1 + off.x, b.y1 + off.y).inflate(-0.5, -0.5);
        !others.iter().any(|o| o.x0 < r.x1 && r.x0 < o.x1 && o.y0 < r.y1 && r.y0 < o.y1)
    };
    let base = d * along;
    [0.0, 1.0, -1.0, 2.0, -2.0, 3.0, -3.0, 4.0, -4.0].iter().map(|k| base + perp * (k * across)).find(|o| free(*o)).or(Some(base))
}

/// `connector.next`: a copy of a shape on one side of it (its text left behind), joined to it by
/// an elbowed connector, and selected.
fn next(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "connector.next";
    let doc = s.doc()?;
    let src = match id(p, "id") {
        Some(i) => i,
        None => match s.selection().as_slice() {
            [one] => figcraft_doc::shape_text::click_target(doc, *one),
            _ => return Err(bad(cmd, "select one shape")),
        },
    };
    let n = doc.get(src)?;
    if n.is_page() || matches!(n.kind, NodeKind::Connector { .. } | NodeKind::Section { .. }) || n.text_host.is_some() {
        return Err(bad(cmd, "adds next to a shape (not a page, section, connector or shape text)"));
    }
    let side = match string(p, "side") {
        Some(v) => Magnet::parse(&v).filter(|m| side_dir(*m).is_some()).ok_or_else(|| bad(cmd, "`side` is top|right|bottom|left"))?,
        None => Magnet::Right,
    };
    let off = next_offset(doc, src, side).ok_or_else(|| bad(cmd, "this layer has no box to add beside"))?;
    let parent = n.parent.ok_or_else(|| bad(cmd, "a page has no side"))?;
    let doc = s.doc_mut()?;
    let index = doc.index_in_parent(src).map(|i| i + 1);
    let copy = doc.duplicate_into(src, parent, index)?;
    super::arrange::move_world(doc, copy, off.x, off.y)?;
    let page = doc.page_of(src).unwrap_or(parent);
    let cid = doc.alloc_id();
    let opposite = match side {
        Magnet::Top => Magnet::Bottom,
        Magnet::Right => Magnet::Left,
        Magnet::Bottom => Magnet::Top,
        _ => Magnet::Right,
    };
    let node = Node::new_connector(cid, "Connector", ConnectorEnd::attached(src, side), ConnectorEnd::attached(copy, opposite));
    doc.insert(node, page, None)?;
    figcraft_doc::connector::reroute(doc, cid);
    s.set_selection(vec![copy]);
    Ok(json!({"id": copy, "connector": cid}))
}
