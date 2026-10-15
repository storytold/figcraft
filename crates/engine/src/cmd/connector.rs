//! Connectors: lines whose ends attach to layers and follow them (the routing is
//! `figcraft_doc::connector`; [`crate::normalize`] re-routes them after every edit).

use figcraft_doc::{ConnectorEnd, ConnectorLineType, Document, Magnet, Node, NodeId, NodeKind};
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
