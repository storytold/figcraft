//! Dev Mode annotations: notes designers attach to layers for developers, each with optional
//! pinned properties (width, fill, font size…) that Dev Mode shows with their current values and
//! an optional category (Development, Accessibility… or one of the file's own, each with a colour).
//! They are stored on the layer (`Node::annotations`), saved with the file and undoable; the
//! categories are stored with the document (`Document::annotation_categories`, left out while
//! they're the defaults).

use std::sync::LazyLock;

use figcraft_doc::{Annotation, AnnotationCategory, Document, NodeId};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::devinspect::{Prop, Section, properties};
use crate::params::{color_of, get, id, req_index, req_string, string, targets};
use crate::{bad, spec};

/// Most annotations on one layer.
pub const MAX_PER_LAYER: usize = 64;
/// Longest annotation text (chars).
pub const MAX_LABEL: usize = 5_000;
/// Most annotated layers listed or drawn for one page.
pub const MAX_LISTED: usize = 2_000;
/// Most annotation categories in a file.
pub const MAX_CATEGORIES: usize = 64;
/// Longest category name (chars).
pub const MAX_CATEGORY_NAME: usize = 64;

/// The file's annotation categories (the defaults until it changes them).
pub fn categories(doc: &Document) -> &[AnnotationCategory] {
    static DEFAULTS: LazyLock<Vec<AnnotationCategory>> = LazyLock::new(AnnotationCategory::defaults);
    doc.annotation_categories.as_deref().unwrap_or(&DEFAULTS)
}

/// Annotation `a`'s category (`None`: uncategorised, or a category the file no longer has).
pub fn category_of<'d>(doc: &'d Document, a: &Annotation) -> Option<&'d AnnotationCategory> {
    let id = a.category.as_deref()?;
    categories(doc).iter().find(|c| c.id == id)
}

/// The category named by `key`: its id, else its name (ignoring case).
pub fn find_category<'a>(list: &'a [AnnotationCategory], key: &str) -> Option<&'a AnnotationCategory> {
    let key = key.trim();
    list.iter().find(|c| c.id == key).or_else(|| list.iter().find(|c| c.name.eq_ignore_ascii_case(key)))
}

fn category_json(c: &AnnotationCategory) -> Value {
    json!({"id": c.id, "name": c.name, "color": c.color.to_css_hex()})
}

/// A property an annotation can pin: its id, its label and where Dev Mode's property list shows it
/// (a section and a row; `None` = the section's rows other pinnable properties don't claim).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pinnable {
    pub id: &'static str,
    pub label: &'static str,
    section: &'static str,
    row: Option<&'static str>,
}

const fn pin(id: &'static str, label: &'static str, section: &'static str, row: Option<&'static str>) -> Pinnable {
    Pinnable { id, label, section, row }
}

/// Every pinnable property, in the order Dev Mode lists them.
pub const PINNABLE: &[Pinnable] = &[
    pin("width", "Width", "Layout", Some("Width")),
    pin("height", "Height", "Layout", Some("Height")),
    pin("top", "Top", "Layout", Some("Top")),
    pin("left", "Left", "Layout", Some("Left")),
    pin("rotation", "Rotation", "Layout", Some("Rotation")),
    pin("opacity", "Opacity", "Layout", Some("Opacity")),
    pin("cornerRadius", "Corner radius", "Layout", Some("Radius")),
    pin("direction", "Direction", "Auto layout", Some("Direction")),
    pin("gap", "Gap", "Auto layout", Some("Gap")),
    pin("padding", "Padding", "Auto layout", Some("Padding")),
    pin("alignment", "Alignment", "Auto layout", Some("Align")),
    pin("fontFamily", "Font", "Typography", Some("Font")),
    pin("fontStyle", "Font style", "Typography", Some("Style")),
    pin("fontSize", "Font size", "Typography", Some("Size")),
    pin("lineHeight", "Line height", "Typography", Some("Line height")),
    pin("letterSpacing", "Letter spacing", "Typography", Some("Letter spacing")),
    pin("textAlign", "Text align", "Typography", Some("Align")),
    pin("fill", "Fill", "Fills", None),
    pin("stroke", "Stroke", "Strokes", None),
    pin("strokeWeight", "Stroke weight", "Strokes", Some("Weight")),
    pin("effects", "Effects", "Effects", None),
];

/// The pinnable property with id (or label) `key`, ignoring case.
pub fn pinnable(key: &str) -> Option<&'static Pinnable> {
    let key = key.trim();
    PINNABLE.iter().find(|p| p.id.eq_ignore_ascii_case(key) || p.label.eq_ignore_ascii_case(key))
}

/// Whether `n` has anything to show for `p` (a layer without auto layout has no gap…).
pub fn applies(sections: &[Section], p: &Pinnable) -> bool {
    pinned_in(sections, p).is_some()
}

/// `p`'s value in a layer's inspected `sections`, labelled with `p`'s label (`None` when the
/// layer doesn't have it).
pub fn pinned_in(sections: &[Section], p: &Pinnable) -> Option<Prop> {
    let s = sections.iter().find(|s| s.title == p.section)?;
    let rows: Vec<&Prop> = match p.row {
        Some(label) => s.rows.iter().filter(|r| r.label == label).take(1).collect(),
        // The section's own rows: not those another pinnable property shows (strokes' weight).
        None => s.rows.iter().filter(|r| !PINNABLE.iter().any(|q| q.section == p.section && q.row == Some(r.label.as_str()))).collect(),
    };
    let first = rows.first()?;
    let value = match p.row {
        Some(_) => first.value.clone(),
        None => {
            rows.iter().map(|r| if r.label == "Solid" { r.value.clone() } else { format!("{} {}", r.label, r.value) }).collect::<Vec<_>>().join(", ")
        }
    };
    Some(Prop { label: p.label.to_string(), value, color: first.color.clone(), token: first.token.clone() })
}

/// The pinned properties of annotation `a` on layer `id` with their current values (`None` for
/// those the layer doesn't have); unknown ids are skipped.
pub fn pinned(doc: &Document, id: NodeId, a: &Annotation) -> Vec<(&'static Pinnable, Option<Prop>)> {
    if a.properties.is_empty() {
        return Vec::new();
    }
    let sections = properties(doc, id);
    a.properties.iter().filter_map(|k| pinnable(k)).map(|p| (p, pinned_in(&sections, p))).collect()
}

/// The annotated layers on `page`, in layer order (bounded).
pub fn on_page(doc: &Document, page: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![page];
    let mut visited = 0usize;
    while let Some(id) = stack.pop() {
        visited += 1;
        if out.len() >= MAX_LISTED || visited > 200_000 {
            break;
        }
        let Some(n) = doc.node(id) else { continue };
        if !n.annotations.is_empty() {
            out.push(id);
        }
        stack.extend(n.children.iter().rev().copied());
    }
    out
}

/// An annotation as JSON: its layer, index, text and pinned properties with their values.
pub fn annotation_json(doc: &Document, id: NodeId, index: usize, a: &Annotation) -> Value {
    let name = doc.node(id).map(|n| n.name.clone()).unwrap_or_default();
    let props: Vec<Value> = pinned(doc, id, a)
        .into_iter()
        .map(|(p, v)| {
            let mut o = json!({"property": p.id, "label": p.label, "value": v.as_ref().map(|v| v.value.clone())});
            if let Some(t) = v.as_ref().and_then(|v| v.token.clone()) {
                o["token"] = json!(t);
            }
            if let Some(c) = v.and_then(|v| v.color) {
                o["color"] = json!(c);
            }
            o
        })
        .collect();
    let mut o = json!({"id": id, "name": name, "index": index, "label": a.label, "properties": props});
    if let Some(c) = category_of(doc, a) {
        o["category"] = category_json(c);
    }
    o
}

/// `properties` from params: known ids, each once, in the given order.
fn parse_properties(cmd: &str, p: &Value) -> crate::Result<Option<Vec<String>>> {
    let Some(v) = get(p, "properties") else { return Ok(None) };
    let Value::Array(list) = v else { return Err(bad(cmd, "`properties` must be a list of property ids")) };
    let mut out: Vec<String> = Vec::new();
    for item in list.iter().take(PINNABLE.len() * 4) {
        let k = item.as_str().unwrap_or_default();
        let found = pinnable(k).ok_or_else(|| {
            let ids: Vec<&str> = PINNABLE.iter().map(|p| p.id).collect();
            bad(cmd, format!("unknown property {item}; one of: {}", ids.join(", ")))
        })?;
        if !out.iter().any(|o| o == found.id) {
            out.push(found.id.to_string());
        }
    }
    Ok(Some(out))
}

/// `label` from params, trimmed of trailing whitespace and capped.
fn parse_label(cmd: &str, p: &Value) -> crate::Result<Option<String>> {
    match get(p, "label").or_else(|| get(p, "text")) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim_end().chars().take(MAX_LABEL).collect())),
        Some(_) => Err(bad(cmd, "`label` must be a string")),
    }
}

/// `category` from params: `Some(None)` clears it (null or ""), `Some(Some(id))` sets it, `None`
/// leaves it.
fn parse_category(cmd: &str, doc: &Document, p: &Value) -> crate::Result<Option<Option<String>>> {
    match p.get("category") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(Some(None)),
        Some(Value::String(s)) => {
            let list = categories(doc);
            let c = find_category(list, s).ok_or_else(|| {
                let names: Vec<&str> = list.iter().map(|c| c.name.as_str()).collect();
                bad(cmd, format!("no category {s:?}; one of: {}", names.join(", ")))
            })?;
            Ok(Some(Some(c.id.clone())))
        }
        Some(_) => Err(bad(cmd, "`category` must be a category id or name (null: none)")),
    }
}

fn check(cmd: &str, a: &Annotation) -> crate::Result<()> {
    if a.label.trim().is_empty() && a.properties.is_empty() {
        return Err(bad(cmd, "an annotation needs a note or a pinned property"));
    }
    Ok(())
}

/// The layer an annotation command acts on: `id`, else the first selected layer.
fn layer(cmd: &str, s: &crate::Session, p: &Value) -> crate::Result<NodeId> {
    let id = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a layer or pass `id`"))?;
    let n = s.doc()?.node(id).ok_or_else(|| bad(cmd, "no such layer"))?;
    if n.is_page() {
        return Err(bad(cmd, "pages can't be annotated"));
    }
    Ok(id)
}

/// The annotation `index` on `id` must exist.
fn existing(cmd: &str, s: &crate::Session, id: NodeId, p: &Value) -> crate::Result<usize> {
    let index = req_index(cmd, p, "index")?;
    let count = s.doc()?.node(id).map_or(0, |n| n.annotations.len());
    if index >= count {
        return Err(bad(cmd, format!("no annotation {index} (the layer has {count})")));
    }
    Ok(index)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "annotation.add",
            "Add annotation",
            "{id? (default the first selected layer), label? | text? (the note), properties?: [width|height|top|left|rotation|opacity|cornerRadius|direction|gap|padding|alignment|fontFamily|fontStyle|fontSize|lineHeight|letterSpacing|textAlign|fill|stroke|strokeWeight|effects], category? (id or name)} → {id, index}  (a note, pinned properties or both)",
            true,
            |s, p| {
                let cmd = "annotation.add";
                let id = layer(cmd, s, p)?;
                let category = parse_category(cmd, s.doc()?, p)?.flatten();
                let a = Annotation {
                    label: parse_label(cmd, p)?.unwrap_or_default(),
                    properties: parse_properties(cmd, p)?.unwrap_or_default(),
                    category,
                };
                check(cmd, &a)?;
                let n = s.doc_mut()?.get_mut(id)?;
                if n.annotations.len() >= MAX_PER_LAYER {
                    return Err(bad(cmd, format!("a layer has at most {MAX_PER_LAYER} annotations")));
                }
                n.annotations.push(a);
                Ok(json!({"id": id, "index": n.annotations.len() - 1}))
            }
        ),
        spec!(
            "annotation.edit",
            "Edit annotation",
            "{id?, index, label? | text?, properties?: [..] (replaces the pinned properties), category? (id or name; null: none)}",
            true,
            |s, p| {
                let cmd = "annotation.edit";
                let id = layer(cmd, s, p)?;
                let index = existing(cmd, s, id, p)?;
                let (label, props, category) = (parse_label(cmd, p)?, parse_properties(cmd, p)?, parse_category(cmd, s.doc()?, p)?);
                if label.is_none() && props.is_none() && category.is_none() {
                    return Err(bad(cmd, "pass `label`, `properties` and/or `category`"));
                }
                let n = s.doc_mut()?.get_mut(id)?;
                let a = n.annotations.get_mut(index).ok_or_else(|| bad(cmd, "no such annotation"))?;
                if let Some(l) = label {
                    a.label = l;
                }
                if let Some(pr) = props {
                    a.properties = pr;
                }
                if let Some(c) = category {
                    a.category = c;
                }
                check(cmd, a)?;
                Ok(json!({"id": id, "index": index}))
            }
        ),
        spec!("annotation.remove", "Remove annotation", "{id?, index? (omit: every annotation on the layer)}", true, |s, p| {
            let cmd = "annotation.remove";
            let id = layer(cmd, s, p)?;
            let index = match get(p, "index") {
                Some(_) => Some(existing(cmd, s, id, p)?),
                None => None,
            };
            let n = s.doc_mut()?.get_mut(id)?;
            if n.annotations.is_empty() {
                return Err(bad(cmd, "the layer has no annotations"));
            }
            match index {
                Some(i) if i < n.annotations.len() => {
                    n.annotations.remove(i);
                }
                Some(_) => return Err(bad(cmd, "no such annotation")),
                None => n.annotations.clear(),
            }
            Ok(json!({"id": id, "remaining": n.annotations.len()}))
        }),
        spec!(
            "annotation.list",
            "List annotations",
            "{ids? | id? (those layers), page? (page id, or \"all\"; default the current page), category? (id or name, or \"none\": only those)} → {annotations: [{id, name, index, label, category?: {id, name, color}, properties: [{property, label, value (null when the layer has none), token?, color?}]}]}",
            false,
            |s, p| {
                let doc = s.doc()?;
                // Only one category's (`Some(None)`: the uncategorised).
                let only: Option<Option<String>> = match string(p, "category") {
                    None => None,
                    Some(k) if k.eq_ignore_ascii_case("none") => Some(None),
                    Some(k) => Some(Some(
                        find_category(categories(doc), &k)
                            .map(|c| c.id.clone())
                            .ok_or_else(|| bad("annotation.list", format!("no category {k:?}")))?,
                    )),
                };
                let shown = |a: &Annotation| only.as_ref().is_none_or(|want| category_of(doc, a).map(|c| c.id.as_str()) == want.as_deref());
                let layers: Vec<NodeId> = if get(p, "ids").is_some() || get(p, "id").is_some() {
                    targets(s, p).into_iter().filter(|i| doc.contains(*i)).collect()
                } else if string(p, "page").is_some_and(|v| v.eq_ignore_ascii_case("all")) {
                    doc.pages.iter().flat_map(|pg| on_page(doc, *pg)).take(MAX_LISTED).collect()
                } else {
                    let page = id(p, "page").unwrap_or(s.page()?);
                    if !doc.pages.contains(&page) {
                        return Err(bad("annotation.list", format!("{page} is not a page")));
                    }
                    on_page(doc, page)
                };
                let list: Vec<Value> = layers
                    .iter()
                    .filter_map(|i| doc.node(*i).map(|n| (i, n)))
                    .flat_map(|(i, n)| n.annotations.iter().enumerate().filter(|(_, a)| shown(a)).map(move |(k, a)| annotation_json(doc, *i, k, a)))
                    .collect();
                Ok(json!({"annotations": list}))
            }
        ),
        spec!(
            "annotation.categories",
            "Annotation categories",
            "{} → {categories: [{id, name, color, count (annotations in it, every page)}]}",
            false,
            |s, _| {
                let doc = s.doc()?;
                let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
                for c in doc.nodes().flat_map(|n| n.annotations.iter()).filter_map(|a| a.category.as_deref()) {
                    *counts.entry(c).or_default() += 1;
                }
                let list: Vec<Value> = categories(doc)
                    .iter()
                    .map(|c| {
                        let mut o = category_json(c);
                        o["count"] = json!(counts.get(c.id.as_str()).copied().unwrap_or(0));
                        o
                    })
                    .collect();
                Ok(json!({"categories": list}))
            }
        ),
        spec!("annotation.addCategory", "Add annotation category", "{name, color? (default the first unused colour)} → {id}", true, |s, p| {
            let cmd = "annotation.addCategory";
            let mut list = categories(s.doc()?).to_vec();
            if list.len() >= MAX_CATEGORIES {
                return Err(bad(cmd, format!("a file has at most {MAX_CATEGORIES} categories")));
            }
            let name = category_name(cmd, &list, None, p)?.ok_or_else(|| bad(cmd, "`name` required"))?;
            let color = match category_color(cmd, p)? {
                Some(c) => c,
                None => {
                    let used = |c: &figcraft_color::Color| list.iter().any(|x| x.color.to_hex() == c.to_hex());
                    let fallback = AnnotationCategory::COLORS
                        .get(list.len() % AnnotationCategory::COLORS.len())
                        .copied()
                        .unwrap_or(figcraft_color::Color::BLACK);
                    AnnotationCategory::COLORS.iter().find(|c| !used(c)).copied().unwrap_or(fallback)
                }
            };
            let id = (1..=MAX_CATEGORIES + 1).map(|k| format!("custom-{k}")).find(|i| !list.iter().any(|c| c.id == *i)).unwrap_or_default();
            list.push(AnnotationCategory { id: id.clone(), name, color });
            s.doc_mut()?.annotation_categories = Some(list);
            Ok(json!({"id": id}))
        }),
        spec!("annotation.editCategory", "Edit annotation category", "{id (id or name), name?, color?}  (rename and/or recolour)", true, |s, p| {
            let cmd = "annotation.editCategory";
            let mut list = categories(s.doc()?).to_vec();
            let key = req_string(cmd, p, "id")?;
            let k = find_category(&list, &key)
                .and_then(|c| list.iter().position(|x| x.id == c.id))
                .ok_or_else(|| bad(cmd, format!("no category {key:?}")))?;
            let (name, color) = (category_name(cmd, &list, Some(k), p)?, category_color(cmd, p)?);
            if name.is_none() && color.is_none() {
                return Err(bad(cmd, "pass `name` and/or `color`"));
            }
            let c = list.get_mut(k).ok_or_else(|| bad(cmd, "no such category"))?;
            if let Some(n) = name {
                c.name = n;
            }
            if let Some(col) = color {
                c.color = col;
            }
            let id = c.id.clone();
            s.doc_mut()?.annotation_categories = Some(list);
            Ok(json!({"id": id}))
        }),
        spec!(
            "annotation.removeCategory",
            "Delete annotation category",
            "{id (id or name)} → {id, uncategorized (how many annotations were in it)}  (its annotations stay, without a category)",
            true,
            |s, p| {
                let cmd = "annotation.removeCategory";
                let doc = s.doc()?;
                let mut list = categories(doc).to_vec();
                let key = req_string(cmd, p, "id")?;
                let id = find_category(&list, &key).map(|c| c.id.clone()).ok_or_else(|| bad(cmd, format!("no category {key:?}")))?;
                list.retain(|c| c.id != id);
                let in_it = |a: &Annotation| a.category.as_deref() == Some(id.as_str());
                let layers: Vec<NodeId> = doc.nodes().filter(|n| n.annotations.iter().any(in_it)).map(|n| n.id).collect();
                let doc = s.doc_mut()?;
                doc.annotation_categories = Some(list);
                let mut count = 0usize;
                for l in layers {
                    for a in doc.get_mut(l)?.annotations.iter_mut().filter(|a| in_it(a)) {
                        a.category = None;
                        count += 1;
                    }
                }
                Ok(json!({"id": id, "uncategorized": count}))
            }
        ),
    ]
}

/// `name` from params for category `editing` (`None`: a new one): trimmed, capped, not empty and
/// not another category's.
fn category_name(cmd: &str, list: &[AnnotationCategory], editing: Option<usize>, p: &Value) -> crate::Result<Option<String>> {
    let Some(v) = get(p, "name") else { return Ok(None) };
    let name: String = v.as_str().ok_or_else(|| bad(cmd, "`name` must be a string"))?.trim().chars().take(MAX_CATEGORY_NAME).collect();
    if name.is_empty() {
        return Err(bad(cmd, "a category needs a name"));
    }
    if list.iter().enumerate().any(|(i, c)| Some(i) != editing && c.name.eq_ignore_ascii_case(&name)) {
        return Err(bad(cmd, format!("there is already a category named {name:?}")));
    }
    Ok(Some(name))
}

/// `color` from params, opaque.
fn category_color(cmd: &str, p: &Value) -> crate::Result<Option<figcraft_color::Color>> {
    match get(p, "color") {
        Some(v) => Ok(Some(color_of(v).ok_or_else(|| bad(cmd, "`color` must be a colour (hex or CSS)"))?.with_alpha(1.0))),
        None => Ok(None),
    }
}
