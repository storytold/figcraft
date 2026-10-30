//! Swapping a library: every copy of library A's components, styles and variables that a document
//! uses is re-pointed to the asset of the same name in library B (a design system moved to another
//! library, or a theme library swapped for another). Instances keep their overrides (by layer and
//! property names, as when swapping an instance), styled and bound layers follow, and what B has
//! no match for stays on A and is reported.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::{Document, ImportedComponent, ImportedRecord, LibraryManifest, NodeId, NodeKind, PublishedLibrary, VariableValue};
use serde_json::Value;

use crate::cmd::component::{main_of, swap_instance};
use crate::components::inside_instance;
use crate::library::{asset_root, import, import_styles, import_variables, prune, style_links, swap_props};
use crate::params::id_of;
use crate::{EngineError, Result};

/// What a swap did: how many copies moved to the other library, and the names of those it has no
/// match for (still from the old library).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Swapped {
    pub components: usize,
    pub styles: usize,
    pub variables: usize,
    pub unmatched: Vec<String>,
    /// Nothing of the old library is used any more: it was removed from the document.
    pub removed: bool,
}

/// The main component a layer of A's copy `a_main` maps to in B's copy rooted at `b_root`: the
/// variant of the same name in a set, else the component itself (a set's default variant when B
/// has a set for a plain component).
fn b_main(doc: &Document, a_main: NodeId, a_root: NodeId, b_root: NodeId) -> Option<NodeId> {
    let b = doc.node(b_root)?;
    match (&b.kind, a_main == a_root) {
        (NodeKind::ComponentSet { .. }, false) => {
            let name = &doc.node(a_main)?.name;
            b.children.iter().copied().find(|c| doc.node(*c).is_some_and(|n| n.name == *name && matches!(n.kind, NodeKind::Component { .. })))
        }
        _ => main_of(doc, b_root),
    }
}

/// Local style id → the library's style id, matched by name and type (the rest go to `out.unmatched`).
fn match_styles(doc: &Document, recs: &[ImportedRecord], m: &LibraryManifest, out: &mut Swapped) -> BTreeMap<String, String> {
    let mut keys = BTreeMap::new();
    for r in recs {
        let Some(st) = doc.style(&r.local) else { continue };
        let kind = st.kind.type_name();
        match m.styles.iter().find(|x| x.published && x.name == st.name && x.kind == kind) {
            Some(x) => {
                keys.insert(r.local.clone(), x.id.clone());
            }
            None => out.unmatched.push(format!("style {}", st.name)),
        }
    }
    keys
}

/// (local component root, the library's component id) by name; the rest go to `out.unmatched`.
fn match_components(doc: &Document, records: &[ImportedComponent], m: &LibraryManifest, out: &mut Swapped) -> Vec<(NodeId, NodeId)> {
    let mut pairs = Vec::new();
    for r in records {
        let name = doc.node(r.root).map(|n| n.name.clone()).unwrap_or_default();
        match m.components.iter().find(|a| a.published && a.name == name) {
            Some(a) => pairs.push((r.root, a.id)),
            None => out.unmatched.push(format!("component {name}")),
        }
    }
    pairs
}

/// Swap the copies of library `from` in `doc` to library `to` by names (see the module docs).
pub fn swap_library(doc: &mut Document, from: &str, to: &PublishedLibrary) -> Result<Swapped> {
    let m = &to.manifest;
    if m.id == from {
        return Err(EngineError::Other("that is the same library".into()));
    }
    // Only what the document uses counts (copies nothing uses any more go first).
    prune(doc, from)?;
    let mut out = Swapped::default();
    let store = doc.library_store();
    let outside = |d: &Document, id: NodeId| store.is_none_or(|s| d.page_of(id) != Some(s));

    // Components: A's copies the document's own instances show (those only other copies use go
    // with them), matched by name to B's published components.
    let shown: BTreeSet<NodeId> = doc
        .nodes()
        .filter(|n| outside(doc, n.id) && !inside_instance(doc, n.id))
        .filter_map(|n| match &n.kind {
            NodeKind::Instance { component, .. } => asset_root(doc, *component),
            _ => None,
        })
        .collect();
    let records: Vec<ImportedComponent> = doc.libraries.components.iter().filter(|r| r.library == from && shown.contains(&r.root)).cloned().collect();
    let pairs = match_components(doc, &records, m, &mut out);
    let keys: Vec<NodeId> = pairs.iter().map(|(_, k)| *k).collect();
    let map = if keys.is_empty() { BTreeMap::new() } else { import(doc, to, &keys, false)? };
    // A's mains (each variant) and roots → B's.
    let mut mains: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    let mut roots: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    for (a_root, key) in &pairs {
        let Some(b_root) = map.get(key).copied() else { continue };
        roots.insert(*a_root, b_root);
        let a_mains: Vec<NodeId> = match doc.node(*a_root).map(|n| &n.kind) {
            Some(NodeKind::ComponentSet { .. }) => doc.node(*a_root).map(|n| n.children.clone()).unwrap_or_default(),
            _ => vec![*a_root],
        };
        let before = mains.len();
        for a in a_mains {
            match b_main(doc, a, *a_root, b_root) {
                Some(b) => {
                    mains.insert(a, b);
                }
                None => out.unmatched.push(format!("variant {}", doc.node(a).map(|n| n.name.as_str()).unwrap_or_default())),
            }
        }
        if mains.len() > before {
            out.components += 1;
        }
    }
    let instances: Vec<(NodeId, NodeId)> = doc
        .nodes()
        .filter(|n| outside(doc, n.id) && !inside_instance(doc, n.id))
        .filter_map(|n| match &n.kind {
            NodeKind::Instance { component, .. } => mains.get(component).map(|b| (n.id, *b)),
            _ => None,
        })
        .collect();
    for (inst, b) in instances {
        swap_instance(doc, inst, b).map_err(EngineError::Other)?;
        // Instance swap values naming A's components name B's.
        let swaps = swap_props(doc, b);
        if let Ok(NodeKind::Instance { props, .. }) = doc.get_mut(inst).map(|n| &mut n.kind) {
            for k in swaps {
                if let Some(to) = props.get(&k).and_then(id_of).and_then(|c| mains.get(&c).or_else(|| roots.get(&c))) {
                    props.insert(k, Value::from(to.0));
                }
            }
        }
    }

    // Styles the document's own layers link to: by name and type.
    let linked: BTreeSet<String> = doc.nodes().filter(|n| outside(doc, n.id)).flat_map(|n| style_links(n).cloned()).collect();
    let style_recs: Vec<ImportedRecord> = doc.libraries.styles.iter().filter(|r| r.library == from && linked.contains(&r.local)).cloned().collect();
    let style_keys = match_styles(doc, &style_recs, m, &mut out);
    let imported = import_styles(doc, to, &style_keys.values().cloned().collect(), false);
    let styles: BTreeMap<String, String> = style_keys.iter().filter_map(|(a, k)| imported.get(k).map(|b| (a.clone(), b.clone()))).collect();
    out.styles = styles.len();

    // Variables: by collection and variable name and type; collections and modes by name.
    let coll_name = |d: &Document, id: &str| d.variables.collection_by_id(id).map(|c| c.name.clone()).unwrap_or_default();
    // Variables the document's own layers bind or its own variables alias or calculate from.
    let remote: BTreeSet<String> = doc.libraries.variables.iter().map(|r| r.local.clone()).collect();
    let aliases = doc
        .variables
        .variables
        .iter()
        .filter(|v| !remote.contains(&v.id))
        .flat_map(|v| v.values.values())
        .flat_map(|x| x.references().into_iter().cloned());
    let used: BTreeSet<String> = doc.nodes().filter(|n| outside(doc, n.id)).flat_map(|n| n.bound.values().cloned()).chain(aliases).collect();
    let var_recs: Vec<ImportedRecord> = doc.libraries.variables.iter().filter(|r| r.library == from && used.contains(&r.local)).cloned().collect();
    let lib_coll = |name: &str| to.doc.variables.collections.iter().find(|c| c.name == name && m.collection(&c.id).is_some_and(|x| x.published));
    let mut var_keys: BTreeMap<String, String> = BTreeMap::new();
    for r in &var_recs {
        let Some(v) = doc.variables.get_by_id(&r.local) else { continue };
        let cname = coll_name(doc, &v.collection);
        let found = lib_coll(&cname).and_then(|c| {
            to.doc
                .variables
                .variables
                .iter()
                .find(|x| x.collection == c.id && x.name == v.name && x.kind == v.kind && m.variable(&x.id).is_some_and(|y| y.published))
        });
        match found {
            Some(x) => {
                var_keys.insert(r.local.clone(), x.id.clone());
            }
            None => out.unmatched.push(format!("variable {cname}/{}", v.name)),
        }
    }
    let coll_keys: BTreeMap<String, String> = doc
        .libraries
        .collections
        .iter()
        .filter(|r| r.library == from)
        .filter_map(|r| lib_coll(&coll_name(doc, &r.local)).map(|c| (r.local.clone(), c.id.clone())))
        .collect();
    let vmap = import_variables(doc, to, &var_keys.values().cloned().collect(), &coll_keys.values().cloned().collect(), false);
    let vars: BTreeMap<String, String> = var_keys.iter().filter_map(|(a, k)| vmap.variables.get(k).map(|b| (a.clone(), b.clone()))).collect();
    // A's collection → (B's, A's mode id → B's mode of the same name).
    let mut colls: BTreeMap<String, (String, BTreeMap<String, String>)> = BTreeMap::new();
    for (a, k) in &coll_keys {
        let (Some(b), Some(ac)) = (vmap.collections.get(k), doc.variables.collection_by_id(a)) else { continue };
        let modes = doc
            .variables
            .collection_by_id(b)
            .map(|bc| {
                ac.modes.iter().filter_map(|am| bc.modes.iter().find(|bm| bm.name == am.name).map(|bm| (am.id.clone(), bm.id.clone()))).collect()
            })
            .unwrap_or_default();
        colls.insert(a.clone(), (b.clone(), modes));
    }
    out.variables = vars.len();
    // shortcut: this file's collections extending one of A's stay on A (listed); move them with their
    // overrides (mode ids differ between libraries) when swapping themes needs it.
    let a_colls: BTreeSet<&str> = doc.libraries.collections.iter().filter(|r| r.library == from).map(|r| r.local.as_str()).collect();
    for c in doc.variables.collections.iter().filter(|c| c.extends.as_deref().is_some_and(|e| a_colls.contains(e))) {
        out.unmatched.push(format!("extended collection {}", c.name));
    }

    // Re-point layers outside the library copies, and local variables aliasing A's.
    let ids: Vec<NodeId> = doc.nodes().filter(|n| outside(doc, n.id)).map(|n| n.id).collect();
    for id in ids {
        let Ok(n) = doc.get_mut(id) else { continue };
        for r in [&mut n.styles.fill, &mut n.styles.stroke, &mut n.styles.effect, &mut n.styles.grid, &mut n.styles.text] {
            if let Some(b) = r.as_ref().and_then(|s| styles.get(s)) {
                *r = Some(b.clone());
            }
        }
        if let NodeKind::Text(t) = &mut n.kind {
            t.map_text_styles(|s| Some(styles.get(s).cloned().unwrap_or_else(|| s.to_string())));
        }
        for v in n.bound.values_mut() {
            if let Some(b) = vars.get(v) {
                *v = b.clone();
            }
        }
        n.var_modes = std::mem::take(&mut n.var_modes)
            .into_iter()
            .map(|(c, mode)| match colls.get(&c).and_then(|(b, modes)| modes.get(&mode).map(|bm| (b.clone(), bm.clone()))) {
                Some(moved) => moved,
                None => (c, mode),
            })
            .collect();
    }
    for v in doc.variables.variables.iter_mut().filter(|v| !remote.contains(&v.id)) {
        for r in v.values.values_mut().flat_map(VariableValue::references_mut) {
            if let Some(b) = vars.get(r.as_str()) {
                *r = b.clone();
            }
        }
    }

    // A's copies nothing uses any more go; A goes too once nothing of it is left.
    prune(doc, from)?;
    let libs = &doc.libraries;
    let left = libs.components.iter().any(|r| r.library == from)
        || libs.styles.iter().any(|r| r.library == from)
        || libs.variables.iter().any(|r| r.library == from)
        || libs.collections.iter().any(|r| r.library == from);
    if !left {
        doc.libraries_mut().enabled.retain(|l| l.id != from);
        out.removed = true;
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
