use super::load::{ext, file_stem, import_summary, load_path, load_with, read, rest_images, sibling_variables, variables_param};
use super::*;

/// How many files File ▸ Open recent keeps.
pub const MAX_RECENT: usize = 20;

/// Put `path` at the top of the recent files.
pub(super) fn remember(s: &mut Session, path: &str) {
    let r = &mut s.prefs.recent_files;
    r.retain(|p| p != path);
    r.insert(0, path.to_string());
    r.truncate(MAX_RECENT);
}

pub(super) fn open_recent(s: &mut Session, p: &Value) -> Result<Value> {
    use crate::file_access::{Access, allow, allowed};
    const CMD: &str = "file.openRecent";
    // Confined to automation roots, the recent files outside them are never listed, opened, removed
    // or named: `index` counts the files listed.
    let listed = |r: &String| allowed(r, Access::Read);
    if p.get("clear").and_then(Value::as_bool) == Some(true) {
        s.prefs.recent_files.retain(|r| !listed(r));
    }
    if let Some(gone) = string(p, "remove") {
        s.prefs.recent_files.retain(|r| *r != gone || !listed(r));
    }
    let path = match (string(p, "path"), num(p, "index")) {
        (Some(path), _) => {
            allow(&path, Access::Read)?;
            Some(path)
        }
        (None, Some(i)) => {
            Some(s.prefs.recent_files.iter().filter(|r| listed(r)).nth(i.max(0.0) as usize).cloned().ok_or_else(|| bad(CMD, "no such recent file"))?)
        }
        _ => None,
    };
    if let Some(path) = path {
        if !crate::file_access::exists(&path) {
            s.prefs.recent_files.retain(|r| *r != path);
            return Err(bad(CMD, format!("{path} no longer exists")));
        }
        return open(s, &json!({"path": path}));
    }
    let files: Vec<Value> = s
        .prefs
        .recent_files
        .iter()
        .filter(|r| listed(r))
        .map(|r| {
            let name = std::path::Path::new(r).file_name().map_or_else(|| r.clone(), |f| f.to_string_lossy().into_owned());
            json!({"path": r, "name": name, "exists": crate::file_access::exists(r)})
        })
        .collect();
    Ok(json!({"files": files}))
}

pub(super) fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let variables = variables_param("file.open", p)?;
    if string(p, "path").is_none()
        && let Some(d) = string(p, "data")
    {
        return open_data(s, &d, p, variables.as_ref());
    }
    let path = req_string("file.open", p, "path")?;
    // Already open: switch to it, and say when the file changed on disk since this tab read or
    // saved it (`stale`), or read it again (`reload`).
    if let Some(i) = s.docs.iter().position(|d| d.path.as_deref() == Some(path.as_str())) {
        s.active = i;
        remember(s, &path);
        let stale = s.ds()?.on_disk.as_ref().is_some_and(|read| crate::library::stat(&path).ok().as_ref() != Some(read));
        if !boolean(p, "reload").unwrap_or(false) {
            return Ok(if stale { json!({"index": i, "stale": true}) } else { json!({"index": i}) });
        }
        if s.ds()?.is_dirty() {
            return Err(EngineError::Disabled(
                "file.open".into(),
                "the open copy has unsaved changes: save them, or Revert to saved to drop them".into(),
            ));
        }
        if s.live().doc_uid.is_some() {
            return Err(EngineError::Disabled("file.open".into(), "leave the live session before reading the file again".into()));
        }
        revert(s)?;
        return Ok(json!({"index": i, "reloaded": true}));
    }
    // Stamped before reading: a change made during the read makes the next save refuse.
    let on_disk = crate::library::stat(&path).ok();
    let (loaded, rest) = load_path(&path, variables.as_ref())?;
    remember(s, &path);
    let native = !rest && matches!(ext(&path).as_str(), "figcraft" | "json");
    let warnings = rest || matches!(ext(&path).as_str(), "svg" | "penpot");
    // A native file that lost something (a damaged image, say): the app says so, not only the result.
    if (native || warnings) && !loaded.issues.is_empty() {
        s.message = Some(match loaded.issues.as_slice() {
            [one] => format!("{}: {one}", file_stem(&path)),
            more => format!("{}: {} problems were repaired or left out while opening it, e.g. {}", file_stem(&path), more.len(), more[0]),
        });
    }
    let summary = import_summary(&loaded.doc);
    let images_left_out = if native { loaded.images_past_budget } else { 0 };
    let mut ds = DocState::new(loaded.doc, native.then(|| path.clone()))
        .with_versions(loaded.versions)
        .with_branches(loaded.branches)
        .with_images_left_in_file(&path, images_left_out);
    ds.on_disk = on_disk.filter(|_| native);
    s.docs.push(ds);
    s.active = s.docs.len() - 1;
    let mut out = json!({"index": s.active, "summary": summary});
    out[if warnings { "warnings" } else { "repairs" }] = json!(loaded.issues);
    Ok(out)
}

/// Replace the active tab's document with its file as saved (`file.revert`, `file.open {reload}`).
pub(super) fn revert(s: &mut Session) -> Result<()> {
    let path = s.ds()?.path.clone().ok_or_else(|| EngineError::Disabled("file.revert".into(), "the file was never saved".into()))?;
    // Stamped before reading: a change made during the read makes the next save refuse.
    let on_disk = crate::library::stat(&path).ok();
    let (loaded, _) = load_path(&path, None)?;
    let i = s.active;
    let uid = s.ds()?.uid;
    if let Some(slot) = s.docs.get_mut(i) {
        *slot = DocState::new(loaded.doc, Some(path.clone()))
            .with_versions(loaded.versions)
            .with_branches(loaded.branches)
            .with_images_left_in_file(&path, loaded.images_past_budget);
        slot.on_disk = on_disk;
    }
    // Edits on its branch tabs are reverted too.
    crate::branches::close_tabs(s, uid, None);
    Ok(())
}

/// [`open`] for a file given as base64 `data` (a file dropped without a path on disk): `name` tells
/// the format. It opens as a new document with no path, so the first save asks where to put it.
fn open_data(s: &mut Session, data: &str, p: &Value, variables: Option<&Value>) -> Result<Value> {
    let name = req_string("file.open", p, "name")?;
    let data = data.split_once(',').map_or(data, |(_, b)| b);
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| bad("file.open", e.to_string()))?;
    let (loaded, rest) = load_with(&bytes, &name, variables, &figcraft_format::rest::no_images)?;
    let warnings = rest || matches!(ext(&name).as_str(), "svg" | "penpot");
    let summary = import_summary(&loaded.doc);
    s.docs.push(DocState::new(loaded.doc, None).with_versions(loaded.versions).with_branches(loaded.branches));
    s.active = s.docs.len() - 1;
    let mut out = json!({"index": s.active, "summary": summary});
    out[if warnings { "warnings" } else { "repairs" }] = json!(loaded.issues);
    Ok(out)
}

pub(super) fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let path = string(p, "path");
    let variables = variables_param("file.import", p)?;
    let (bytes, name) = if let Some(path) = &path {
        (read(path)?, path.clone())
    } else if let Some(d) = string(p, "data") {
        let d = d.split_once(',').map(|(_, b)| b.to_string()).unwrap_or(d);
        (
            base64::engine::general_purpose::STANDARD.decode(d.trim()).map_err(|e| bad("file.import", e.to_string()))?,
            string(p, "name").unwrap_or_else(|| "import.png".into()),
        )
    } else {
        return Err(bad("file.import", "pass `path` or `data`"));
    };
    let page = s.page()?;
    let (x, y) = (num(p, "x"), num(p, "y"));
    let center = s.ds().map(|d| (d.view.cx, d.view.cy)).unwrap_or((0.0, 0.0));
    match ext(&name).as_str() {
        "svg" => {
            let doc = s.doc_mut()?;
            let (x, y) = (x.unwrap_or(center.0), y.unwrap_or(center.1));
            let imported =
                figcraft_svg::import_svg_with_notes(doc, &bytes, page, x, y, &file_stem(&name)).map_err(|e| EngineError::Other(e.to_string()))?;
            let id = imported.frame;
            // Centred on the point (a paste lands in the middle of the view).
            if boolean(p, "center").unwrap_or(false)
                && let Some(n) = doc.node_mut(id)
            {
                n.transform = figcraft_geom::Affine::translate(((x - n.width / 2.0).round(), (y - n.height / 2.0).round()));
            }
            s.set_selection(vec![id]);
            let mut out = json!({"id": id});
            if !imported.notes.is_empty() {
                out["warnings"] = json!(imported.notes);
            }
            Ok(out)
        }
        "figcraft" | "json" | "fig" | "sketch" | "penpot" => {
            let resolver = path.as_deref().map(rest_images);
            let images: figcraft_format::rest::ImageResolver = match &resolver {
                Some(r) => r,
                None => &figcraft_format::rest::no_images,
            };
            let sibling = if variables.is_none() { path.as_deref().and_then(sibling_variables) } else { None };
            let (other, rest) = load_with(&bytes, &name, variables.as_ref().or(sibling.as_ref()), images)?;
            let summary = import_summary(&other.doc);
            let pages = append_pages(s.doc_mut()?, &other.doc)?;
            if !other.issues.is_empty() {
                s.message = Some(match other.issues.as_slice() {
                    [one] => format!("{}: {one}", file_stem(&name)),
                    more => format!("{}: {} import warnings; first: {}", file_stem(&name), more.len(), more[0]),
                });
            }
            let mut out = json!({"pages": pages, "summary": summary});
            if rest || !other.issues.is_empty() {
                out["warnings"] = json!(other.issues);
            }
            Ok(out)
        }
        _ => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            crate::cmd::create::place_image(
                s,
                &json!({"data": b64, "x": x.unwrap_or(center.0), "y": y.unwrap_or(center.1), "name": file_stem(&name)}),
            )
        }
    }
}

/// Append `other`'s pages (with their settings and flows, its images, styles and variables) to
/// `doc`. Ids are remapped across all pages at once, so instances keep their main, instance layers
/// their source and interactions and flows their targets wherever those are. Styles, variables and
/// collections whose id the document already has are kept as they are.
pub(super) fn append_pages(doc: &mut Document, other: &Document) -> Result<Vec<NodeId>> {
    for (k, v) in &other.images {
        doc.images.entry(k.clone()).or_insert_with(|| v.clone());
    }
    for st in &other.styles {
        if !doc.styles.iter().any(|s| s.id == st.id) {
            doc.styles.push(st.clone());
        }
    }
    let have: std::collections::BTreeSet<String> = doc.variables.collections.iter().map(|c| c.id.clone()).collect();
    doc.variables.collections.extend(other.variables.collections.iter().filter(|c| !have.contains(&c.id)).cloned());
    let have: std::collections::BTreeSet<String> = doc.variables.variables.iter().map(|v| v.id.clone()).collect();
    doc.variables.variables.extend(other.variables.variables.iter().filter(|v| !have.contains(&v.id)).cloned());
    let mut map = std::collections::BTreeMap::new();
    for pg in &other.pages {
        for id in other.subtree(*pg).into_iter().skip(1) {
            map.insert(id, doc.alloc_id());
        }
    }
    // Library copies a .fig import keeps (cached definitions) have no library record to follow:
    // the layers of its store come along into the store of `doc`. Recorded copies are left to
    // `merge_store`, which shares them with copies `doc` already has.
    let cached: Vec<NodeId> = other
        .library_store()
        .and_then(|s| other.node(s))
        .map(|s| s.children.iter().copied().filter(|root| other.libraries.import_of(*root).is_none()).collect())
        .unwrap_or_default();
    for root in &cached {
        for id in other.subtree(*root) {
            map.insert(id, doc.alloc_id());
        }
    }
    let mut new_pages = Vec::new();
    for pg in &other.pages {
        let Some(pn) = other.node(*pg) else { continue };
        let np = doc.add_page(&pn.name);
        let mut kind = pn.kind.clone();
        if let NodeKind::Page { flows, .. } = &mut kind {
            flows.retain_mut(|f| map.get(&f.start).map(|n| f.start = *n).is_some());
        }
        if let Some(n) = doc.node_mut(np) {
            n.kind = kind;
        }
        for c in &pn.children {
            let nodes: Vec<Node> = other.subtree(*c).into_iter().filter_map(|i| other.node(i).cloned()).collect();
            doc.paste_nodes(&nodes, &map, *c, np, None)?;
        }
        new_pages.push(np);
    }
    if !cached.is_empty() {
        let store = doc.ensure_library_store();
        for root in &cached {
            let nodes: Vec<Node> = other.subtree(*root).into_iter().filter_map(|i| other.node(i).cloned()).collect();
            doc.paste_nodes(&nodes, &map, *root, store, None)?;
        }
    }
    for new in map.values() {
        let src = doc.node(*new).and_then(|n| n.source).and_then(|s| map.get(&s).copied());
        if let (Some(src), Some(n)) = (src, doc.node_mut(*new)) {
            n.source = Some(src);
        }
    }
    crate::library::merge_store(doc, other, &map)?;
    Ok(new_pages)
}
