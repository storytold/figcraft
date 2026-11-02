//! Fonts a document names that this machine doesn't have (#752): text layers, their styled
//! ranges, text styles and the string variables bound to a font family. Text in such a family
//! is drawn in a fallback font, so the user is told which ones are missing. Once such a font is
//! there, [`refit`] lets auto-sizing text measure its box in it again.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::{Document, NodeId, NodeKind, StyleKind, VariableValue, Variables};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params::{get, req_targets};
use crate::{Result, Session, spec};

/// A family the document names that no installed or built-in font provides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missing {
    /// As the document spells it.
    pub family: String,
    /// Text layers that use it (0: only a text style or a variable names it).
    pub layers: usize,
    /// The font store source it can be downloaded from (`Google Fonts`, `Noto`; desktop only).
    pub source: Option<String>,
}

/// The families `doc` names that the font database doesn't have, by name. On the desktop the
/// installed fonts are indexed in the background: ask once that's done
/// (`figcraft_text::installed_fonts_known`), or every installed family counts as missing.
pub fn missing(doc: &Document) -> Vec<Missing> {
    // Lower-case family → (spelling, layers).
    let mut used: BTreeMap<String, (String, usize)> = BTreeMap::new();
    let mut add = |family: &str, layer: bool| {
        let family = family.trim();
        if family.is_empty() {
            return;
        }
        let e = used.entry(family.to_lowercase()).or_insert_with(|| (family.to_string(), 0));
        e.1 += usize::from(layer);
    };
    for n in doc.nodes() {
        if let NodeKind::Text(t) = &n.kind {
            let mut families: Vec<&str> = t.spans().into_iter().map(|(_, _, st)| st.font_family.as_str()).collect();
            families.push(&t.style.font_family);
            families.sort_by_key(|f| f.trim().to_lowercase());
            families.dedup_by_key(|f| f.trim().to_lowercase());
            for f in families {
                add(f, true);
            }
        }
        if let Some(var) = n.bound.get("fontFamily") {
            for f in string_values(&doc.variables, var) {
                add(&f, false);
            }
        }
    }
    for st in &doc.styles {
        if let StyleKind::Text { style } = &st.kind {
            add(&style.font_family, false);
        }
        if let Some(var) = st.bound.get("fontFamily") {
            for f in string_values(&doc.variables, var) {
                add(&f, false);
            }
        }
    }
    used.into_values()
        .filter(|(family, _)| !figcraft_text::has_family(family))
        .map(|(family, layers)| Missing { source: source(&family), family, layers })
        .collect()
}

/// A family's download ([`downloads`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Download {
    /// Bytes downloaded of all its files.
    Running { done: u64, total: u64 },
    /// Installed and scanned: the next `font.download` of it fits the open document's text.
    Installed,
    /// Why it failed (nothing was installed).
    Failed(String),
}

#[cfg(not(target_arch = "wasm32"))]
pub use download::{allow_downloads, downloads, downloads_allowed, source};

/// The browser keeps its built-in fonts: nothing is downloaded there.
#[cfg(target_arch = "wasm32")]
mod no_download {
    pub fn source(_: &str) -> Option<String> {
        None
    }
    pub fn downloads() -> Vec<(String, super::Download)> {
        Vec::new()
    }
    pub fn downloads_allowed() -> bool {
        false
    }
    pub fn allow_downloads() -> std::result::Result<(), String> {
        Err("fonts aren't downloaded in the browser".into())
    }
}
#[cfg(target_arch = "wasm32")]
pub use no_download::{allow_downloads, downloads, downloads_allowed, source};

/// Every string value of variable `id` in any mode, following aliases (each variable once).
fn string_values(vars: &Variables, id: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut todo = vec![id.to_string()];
    while let Some(id) = todo.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(var) = vars.get_by_id(&id) else { continue };
        for v in var.values.values() {
            match v {
                VariableValue::String(s) => out.push(s.clone()),
                VariableValue::Alias(a) => todo.push(a.clone()),
                _ => {}
            }
        }
    }
    out
}

/// Auto-sizing text layers measure their boxes again: a box measured while its family was missing
/// (in a fallback font) follows the real face once that's installed or loaded. Only layers under
/// `under` (every layer: `None`) that use one of `families` (any family: `None`). Returns the
/// layers whose size changed.
pub fn refit(doc: &mut Document, under: Option<&[NodeId]>, families: Option<&[String]>) -> Vec<NodeId> {
    let want: Option<BTreeSet<String>> = families.map(|f| f.iter().map(|x| x.trim().to_lowercase()).collect());
    let scope: Option<BTreeSet<NodeId>> = under.map(|ids| ids.iter().flat_map(|i| doc.subtree(*i)).collect());
    let ids: Vec<NodeId> = doc
        .nodes()
        .filter(|n| scope.as_ref().is_none_or(|s| s.contains(&n.id)))
        .filter(|n| {
            let NodeKind::Text(t) = &n.kind else { return false };
            let uses = |w: &BTreeSet<String>| {
                w.contains(&t.style.font_family.trim().to_lowercase())
                    || t.spans().iter().any(|(_, _, st)| w.contains(&st.font_family.trim().to_lowercase()))
            };
            want.as_ref().is_none_or(uses)
                && figcraft_text::auto_size(n).is_some_and(|(w, h)| (w - n.width).abs() > 1e-6 || (h - n.height).abs() > 1e-6)
        })
        .map(|n| n.id)
        .collect();
    if !ids.is_empty() {
        let before = doc.clone();
        doc.mark_dirty(ids.iter().copied());
        crate::normalize(doc, &before);
    }
    ids
}

/// [`refit`] in the open document, if there is one.
pub(crate) fn refit_open(s: &mut Session, under: Option<&[NodeId]>, families: Option<&[String]>) -> Vec<NodeId> {
    match s.doc_mut() {
        Ok(doc) => refit(doc, under, families),
        Err(_) => Vec::new(),
    }
}

/// `text.refit`: the layers `ids` names (an `ids` that isn't a list of layers is refused), else the
/// whole document.
fn refit_text(s: &mut Session, p: &Value) -> Result<Value> {
    let under = match get(p, "ids") {
        None | Some(Value::Null) => None,
        Some(_) => Some(req_targets("text.refit", s, p)?),
    };
    s.doc()?;
    Ok(json!({"refitted": refit_open(s, under.as_deref(), None)}))
}

fn missing_fonts(s: &mut Session, _: &Value) -> Result<Value> {
    let families: Vec<Value> = missing(s.doc()?).into_iter().map(|m| json!({"family": m.family, "layers": m.layers, "source": m.source})).collect();
    s.request_ui("dialog.missingFonts", json!({}));
    Ok(json!({"families": families}))
}

/// The font commands: `text.missingFonts`, `text.refit`, and on the desktop [`download`]'s.
pub fn specs() -> Vec<CommandSpec> {
    #[allow(unused_mut)]
    let mut list = vec![
        spec!(
            "text.missingFonts",
            "Missing fonts",
            "{}  fonts the open document names that aren't installed (text layers, styled ranges, text styles, font family variables); the app shows them → {families: [{family, layers, source (the font store source font.download gets it from, or null)}]}",
            false,
            missing_fonts
        ),
        spec!(
            "text.refit",
            "Fit text to fonts",
            "{ids? (these layers and the text in them; default: the whole document)}  auto-sizing text measures its box again, e.g. a box measured in a fallback font while its family was missing → {refitted: [ids whose size changed]}",
            true,
            refit_text
        ),
    ];
    #[cfg(not(target_arch = "wasm32"))]
    list.extend(download::specs());
    list
}

#[cfg(not(target_arch = "wasm32"))]
pub mod download;

#[cfg(test)]
mod tests;
