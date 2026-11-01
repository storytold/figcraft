//! Pages.

use figcraft_doc::proto::Flow;
use figcraft_doc::{Guide, NodeId, NodeKind};
use serde_json::{Value, json};

use super::{CommandSpec, ok};
use crate::params::{boolean, color, id, req_index, req_num, req_string, string};
use crate::{Session, bad, spec};

/// After a page edit: the file keeps a page that isn't a divider, and the canvas shows one (the
/// first) when its page was deleted or became a divider.
fn settle(s: &mut Session, cmd: &str) -> crate::Result<()> {
    let doc = s.doc()?;
    let Some(open) = doc.first_open_page() else { return Err(bad(cmd, "a file needs a page that isn't a divider")) };
    let page = s.page()?;
    if !doc.pages.contains(&page) || doc.is_divider(page) {
        let ds = s.ds_mut()?;
        ds.page = open;
        ds.selection.clear();
        ds.text_edit = None;
        ds.zoom_to_fit();
    }
    Ok(())
}

/// The page a command targets: the explicit `id` (which must be a page) or the open page.
fn page_target(s: &Session, cmd: &str, p: &Value) -> crate::Result<NodeId> {
    let Some(target) = id(p, "id") else { return s.page() };
    if !s.doc()?.pages.contains(&target) {
        return Err(bad(cmd, format!("{target} is not a page")));
    }
    Ok(target)
}

/// The page and ruler guide commands.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("page.add", "Add page", "{name?} (a name starting with a hyphen adds a divider)", true, |s, p| {
            // An added page takes the current page's colour; the theme only decides new files.
            let current = s.page().ok().and_then(|p| match s.doc().ok()?.node(p)?.kind {
                NodeKind::Page { background, .. } => Some(background),
                _ => None,
            });
            let background = current.unwrap_or_else(|| s.new_page_background());
            let doc = s.doc_mut()?;
            let name = string(p, "name").unwrap_or_else(|| format!("Page {}", doc.pages.len() + 1));
            let id = doc.add_page(&name);
            if let NodeKind::Page { background: b, .. } = &mut doc.get_mut(id)?.kind {
                *b = background;
            }
            if doc.is_divider(id) {
                return Ok(json!({"id": id, "divider": true}));
            }
            let ds = s.ds_mut()?;
            ds.page = id;
            ds.selection.clear();
            ds.zoom_to_fit();
            Ok(json!({"id": id}))
        }),
        spec!("page.select", "Go to page", "{id | index}", false, |s, p| {
            let doc = s.doc()?;
            let id = match id(p, "id") {
                Some(i) => i,
                None => {
                    let k = req_index("page.select", p, "index")?;
                    *doc.pages.get(k).ok_or_else(|| bad("page.select", "no such page"))?
                }
            };
            if !doc.pages.contains(&id) {
                return Err(bad("page.select", format!("{id} is not a page")));
            }
            if doc.is_divider(id) {
                return Err(bad("page.select", "a divider isn't a page to open"));
            }
            let ds = s.ds_mut()?;
            ds.page = id;
            ds.selection.clear();
            ds.text_edit = None;
            ds.zoom_to_fit();
            ok()
        }),
        spec!("page.rename", "Rename page", "{id?, name} (an empty page named with a leading hyphen becomes a divider)", true, |s, p| {
            let name = req_string("page.rename", p, "name")?;
            let id = page_target(s, "page.rename", p)?;
            s.doc_mut()?.get_mut(id)?.name = name;
            settle(s, "page.rename")?;
            ok()
        }),
        spec!("page.delete", "Delete page", "{id?}", true, |s, p| {
            let id = page_target(s, "page.delete", p)?;
            s.doc_mut()?.remove(id)?;
            settle(s, "page.delete")?;
            ok()
        }),
        spec!("page.duplicate", "Duplicate page", "{id?}", true, |s, p| {
            let src = page_target(s, "page.duplicate", p)?;
            let doc = s.doc_mut()?;
            let name = format!("Copy of {}", doc.get(src)?.name);
            let np = doc.add_page(&name);
            // One copy with one id map, so navigation between the copied frames points at the copies;
            // then each layer and its copy (also those in sections), so flows can follow their start frames.
            let children = doc.get(src)?.children.clone();
            let copied = doc.duplicate_all_into(&children, np)?;
            let mut copies = Vec::new();
            for (c, copy) in children.iter().zip(&copied) {
                copies.extend(doc.subtree(*c).into_iter().zip(doc.subtree(*copy)));
            }
            // The copy keeps the page's colour, flows (pointing at the copied frames), prototype settings and guides.
            let guides = doc.get(src)?.guides.clone();
            doc.get_mut(np)?.guides = guides;
            if let NodeKind::Page { background, flows, prototype } = doc.get(src)?.kind.clone()
                && let NodeKind::Page { background: b, flows: f, prototype: pr } = &mut doc.get_mut(np)?.kind
            {
                *b = background;
                *f = flows
                    .into_iter()
                    .filter_map(|fl| copies.iter().find(|(old, _)| *old == fl.start).map(|(_, new)| Flow { start: *new, ..fl }))
                    .collect();
                *pr = prototype;
            }
            // Keep the new page right after the source.
            if let Some(si) = doc.pages.iter().position(|x| *x == src) {
                doc.pages.retain(|x| *x != np);
                doc.pages.insert(si + 1, np);
            }
            s.ds_mut()?.page = np;
            Ok(json!({"id": np}))
        }),
        spec!("page.move", "Move page", "{id?, index}", true, |s, p| {
            let id = id(p, "id").unwrap_or(s.page()?);
            let k = req_num("page.move", p, "index")?.max(0.0) as usize;
            let doc = s.doc_mut()?;
            if !doc.pages.contains(&id) {
                return Err(bad("page.move", format!("{id} is not a page")));
            }
            doc.pages.retain(|x| *x != id);
            let k = k.min(doc.pages.len());
            doc.pages.insert(k, id);
            ok()
        }),
        spec!("page.setBackground", "Page background", "{id?, color}", true, |s, p| {
            let c = color(p, "color").ok_or_else(|| bad("page.setBackground", "`color` required"))?;
            let id = page_target(s, "page.setBackground", p)?;
            if let NodeKind::Page { background, .. } = &mut s.doc_mut()?.get_mut(id)?.kind {
                *background = c;
            }
            ok()
        }),
        spec!("page.list", "List pages", "{}", false, |s, _| {
            let doc = s.doc()?;
            let cur = s.page()?;
            Ok(Value::Array(
                doc.pages
                    .iter()
                    .filter_map(|p| doc.node(*p))
                    .map(|n| json!({"id": n.id, "name": n.name, "current": n.id == cur, "children": n.children.len(), "divider": doc.is_divider(n.id)}))
                    .collect(),
            ))
        }),
        spec!("guide.add", "Add guide", "{vertical (bool), offset, frame? | page?} → {index}", true, |s, p| {
            let owner = guide_owner(s, "guide.add", p)?;
            let offset = req_num("guide.add", p, "offset")?.clamp(-1e7, 1e7);
            let g = Guide { vertical: boolean(p, "vertical").unwrap_or(true), offset };
            let guides = s.doc_mut()?.get_mut(owner)?.guides_mut();
            if guides.len() >= 10_000 {
                return Err(bad("guide.add", "too many guides"));
            }
            guides.push(g);
            Ok(json!({"index": guides.len() - 1}))
        }),
        spec!("guide.move", "Move guide", "{index, offset, frame? | page?}", true, |s, p| {
            let owner = guide_owner(s, "guide.move", p)?;
            let k = guide_index("guide.move", p)?;
            let offset = req_num("guide.move", p, "offset")?.clamp(-1e7, 1e7);
            s.doc_mut()?.get_mut(owner)?.guides_mut().get_mut(k).ok_or_else(|| bad("guide.move", "no such guide"))?.offset = offset;
            ok()
        }),
        spec!("guide.remove", "Remove guide", "{index?, frame? | page?} (no index: the selected guide)", true, |s, p| {
            let (owner, k) = match p.get("index") {
                None | Some(Value::Null) => s.ds()?.selected_guide().ok_or_else(|| bad("guide.remove", "no guide is selected (pass `index`)"))?,
                Some(_) => (guide_owner(s, "guide.remove", p)?, guide_index("guide.remove", p)?),
            };
            let guides = s.doc_mut()?.get_mut(owner)?.guides_mut();
            if k >= guides.len() {
                return Err(bad("guide.remove", "no such guide"));
            }
            guides.remove(k);
            let ds = s.ds_mut()?;
            match ds.guide {
                Some((o, i)) if o == owner && i == k => ds.guide = None,
                // A later guide of the same owner moves up one place; keep the selection on it.
                Some((o, i)) if o == owner && i > k => ds.guide = Some((o, i - 1)),
                _ => {}
            }
            ok()
        }),
        spec!(
            "guide.select",
            "Select guide",
            "{index?, frame? | page?}  a ruler guide to delete or nudge (deselects layers; no index: none)",
            false,
            |s, p| {
                let picked = match p.get("index") {
                    None | Some(Value::Null) => None,
                    Some(_) => {
                        let owner = guide_owner(s, "guide.select", p)?;
                        let k = guide_index("guide.select", p)?;
                        if k >= s.doc()?.node(owner).map_or(0, |n| n.guides().len()) {
                            return Err(bad("guide.select", "no such guide"));
                        }
                        Some((owner, k))
                    }
                };
                if picked.is_some() {
                    s.set_selection(vec![]);
                }
                let ds = s.ds_mut()?;
                ds.guide = picked;
                ds.revision += 1;
                Ok(match picked {
                    Some((owner, index)) => json!({"owner": owner, "index": index}),
                    None => Value::Null,
                })
            }
        ),
        spec!("guide.clear", "Remove all guides", "{frame? | page?}", true, |s, p| {
            let owner = guide_owner(s, "guide.clear", p)?;
            s.doc_mut()?.get_mut(owner)?.guides_mut().clear();
            ok()
        }),
    ]
}

/// Whose guides a command edits: `frame` (in its own coordinates), else `page`, else the current
/// page.
fn guide_owner(s: &Session, cmd: &str, p: &Value) -> crate::Result<NodeId> {
    let owner = match id(p, "frame") {
        Some(f) => f,
        None => id(p, "page").unwrap_or(s.page()?),
    };
    let doc = s.doc()?;
    match doc.node(owner) {
        Some(_) if crate::guides::keeps_guides(doc, owner) => Ok(owner),
        Some(_) => Err(bad(cmd, "guides go on a page or a top-level frame")),
        None => Err(bad(cmd, "no such page or frame")),
    }
}

/// A guide index from params (never negative).
fn guide_index(cmd: &str, p: &Value) -> crate::Result<usize> {
    req_index(cmd, p, "index")
}
