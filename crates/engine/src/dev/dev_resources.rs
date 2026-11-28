//! Dev resources: links for developers (documentation, code, tickets, design notes…) attached to a
//! layer, each a URL and a name. Dev Mode lists them in the Inspect tab; an instance also shows
//! the links of its main component (and of that component's set), read-only. They are stored on
//! the layer (`Node::dev_resources`: saved, synced, undoable).
//!
//! URLs are data: they are checked (http or https, a host, no spaces or control characters,
//! capped) and never fetched or opened by the engine. A link added without a name is named after
//! its URL (no page title is looked up).

use figcraft_doc::{DevResource, Document, NodeId, NodeKind};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::components::{inside_instance, variant_set};
use crate::params::{get, req_index, targets};
use crate::{Result, Session, bad, spec};

/// Most dev resources on one layer.
pub const MAX_PER_LAYER: usize = 32;
/// Longest URL (bytes).
pub const MAX_URL: usize = 2_048;
/// Longest name (chars).
pub const MAX_NAME: usize = 200;

/// `url` trimmed if it is a web link: http or https, a host, no whitespace or control characters.
pub fn check_url(url: &str) -> std::result::Result<String, String> {
    let url = url.trim();
    if url.len() > MAX_URL {
        return Err(format!("a link is at most {MAX_URL} characters"));
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("a link has no spaces".into());
    }
    let lower = url.to_ascii_lowercase();
    let rest = ["https://", "http://"].iter().find_map(|p| lower.strip_prefix(p)).ok_or("a link starts with https:// or http://")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit('@').next().unwrap_or_default();
    if host.is_empty() || host.starts_with(':') {
        return Err("a link needs a site after https://".into());
    }
    Ok(url.to_string())
}

/// The name a link gets when none is given: its URL without the scheme, `www.` and a trailing
/// slash.
pub fn default_name(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    rest.chars().take(MAX_NAME).collect()
}

/// A dev resource as listed: its own (with its index) or inherited `from` a main component, a
/// component set or, inside an instance, the main's layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub resource: DevResource,
    pub index: Option<usize>,
    pub from: Option<NodeId>,
}

/// Layer `id`'s dev resources: its own first (none for a layer inside an instance, whose links come
/// from the main's layer), then those of its main component and that component's set.
pub fn list(doc: &Document, id: NodeId) -> Vec<Listed> {
    let Some(n) = doc.node(id) else { return Vec::new() };
    let inherited = |from: NodeId| doc.node(from).into_iter().flat_map(move |m| m.dev_resources.iter().map(move |r| (r, from)));
    if inside_instance(doc, id) {
        let from = n.source.unwrap_or(id);
        return n.dev_resources.iter().map(|r| Listed { resource: r.clone(), index: None, from: Some(from) }).collect();
    }
    let mut out: Vec<Listed> = n.dev_resources.iter().enumerate().map(|(i, r)| Listed { resource: r.clone(), index: Some(i), from: None }).collect();
    if let NodeKind::Instance { component, .. } = n.kind {
        let set = variant_set(doc, component);
        for (r, from) in inherited(component).chain(set.into_iter().flat_map(inherited)) {
            out.push(Listed { resource: r.clone(), index: None, from: Some(from) });
        }
    }
    out
}

/// `list` as JSON: `[{name, url, index? (own), from? (inherited)}]`.
pub fn list_json(doc: &Document, id: NodeId) -> Value {
    Value::Array(
        list(doc, id)
            .into_iter()
            .map(|l| {
                let mut j = json!({"name": l.resource.name, "url": l.resource.url});
                if let Some(i) = l.index {
                    j["index"] = json!(i);
                }
                if let Some(f) = l.from {
                    j["from"] = json!(f);
                }
                j
            })
            .collect(),
    )
}

/// The layer a command acts on: not a page, not a layer inside an instance.
fn layer(cmd: &str, s: &Session, p: &Value) -> Result<NodeId> {
    let id = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a layer or pass `id`"))?;
    let doc = s.doc()?;
    let n = doc.node(id).ok_or_else(|| bad(cmd, "no such layer"))?;
    if n.is_page() {
        return Err(bad(cmd, "pages can't have dev resources"));
    }
    if inside_instance(doc, id) {
        return Err(bad(cmd, "this layer is inside an instance: add the link to the main component's layer"));
    }
    Ok(id)
}

/// The `url` parameter, checked.
fn url_param(cmd: &str, p: &Value) -> Result<Option<String>> {
    match get(p, "url") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(u)) => check_url(u).map(Some).map_err(|e| bad(cmd, e)),
        Some(_) => Err(bad(cmd, "url: a string")),
    }
}

/// The `name` parameter, trimmed and clipped (`None` when missing or blank).
fn name_param(cmd: &str, p: &Value) -> Result<Option<String>> {
    match get(p, "name") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(n)) => Ok(Some(n.trim().chars().take(MAX_NAME).collect::<String>()).filter(|n| !n.is_empty())),
        Some(_) => Err(bad(cmd, "name: a string")),
    }
}

/// The own dev resource `index` of `id` must exist.
fn existing(cmd: &str, s: &Session, id: NodeId, p: &Value) -> Result<usize> {
    let index = req_index(cmd, p, "index")?;
    let count = s.doc()?.node(id).map_or(0, |n| n.dev_resources.len());
    if index >= count {
        return Err(bad(cmd, format!("no dev resource {index} (the layer has {count} of its own)")));
    }
    Ok(index)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "devResource.add",
            "Add dev resource",
            "{id? (default the first selected layer), url (http or https), name? (default the URL without its scheme)}  a link for developers on the layer; an instance also shows its main component's → {id, index}",
            true,
            |s, p| {
                let cmd = "devResource.add";
                let id = layer(cmd, s, p)?;
                let url = url_param(cmd, p)?.ok_or_else(|| bad(cmd, "pass the link's `url`"))?;
                let name = name_param(cmd, p)?.unwrap_or_else(|| default_name(&url));
                let n = s.doc_mut()?.get_mut(id)?;
                if n.dev_resources.len() >= MAX_PER_LAYER {
                    return Err(bad(cmd, format!("a layer has at most {MAX_PER_LAYER} dev resources")));
                }
                n.dev_resources.push(DevResource { url, name });
                Ok(json!({"id": id, "index": n.dev_resources.len() - 1}))
            }
        ),
        spec!("devResource.edit", "Edit dev resource", "{id?, index (of the layer's own), url?, name?}  → {id, index}", true, |s, p| {
            let cmd = "devResource.edit";
            let id = layer(cmd, s, p)?;
            let index = existing(cmd, s, id, p)?;
            let (url, name) = (url_param(cmd, p)?, name_param(cmd, p)?);
            if url.is_none() && name.is_none() {
                return Err(bad(cmd, "pass `url` and/or `name`"));
            }
            let n = s.doc_mut()?.get_mut(id)?;
            let r = n.dev_resources.get_mut(index).ok_or_else(|| bad(cmd, "no such dev resource"))?;
            if let Some(u) = url {
                r.url = u;
            }
            if let Some(nm) = name {
                r.name = nm;
            }
            Ok(json!({"id": id, "index": index}))
        }),
        spec!(
            "devResource.remove",
            "Remove dev resource",
            "{id?, index? (omit: every dev resource of the layer's own)}  → {removed}",
            true,
            |s, p| {
                let cmd = "devResource.remove";
                let id = layer(cmd, s, p)?;
                let index = match get(p, "index") {
                    Some(_) => Some(existing(cmd, s, id, p)?),
                    None => None,
                };
                let n = s.doc_mut()?.get_mut(id)?;
                if n.dev_resources.is_empty() {
                    return Err(bad(cmd, "the layer has no dev resources of its own"));
                }
                let removed = match index {
                    Some(i) if i < n.dev_resources.len() => {
                        n.dev_resources.remove(i);
                        1
                    }
                    Some(_) => return Err(bad(cmd, "no such dev resource")),
                    None => std::mem::take(&mut n.dev_resources).len(),
                };
                Ok(json!({"removed": removed}))
            }
        ),
        spec!(
            "devResource.list",
            "List dev resources",
            "{id? (default the first selected layer)}  the layer's links, its own first, then inherited from its main component and that component's set → [{name, url, index? (own), from? (the layer it comes from)}]",
            false,
            |s, p| {
                let id = targets(s, p).first().copied().ok_or_else(|| bad("devResource.list", "select a layer or pass `id`"))?;
                let doc = s.doc()?;
                doc.node(id).ok_or_else(|| bad("devResource.list", "no such layer"))?;
                Ok(list_json(doc, id))
            }
        ),
    ]
}
