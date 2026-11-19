//! Responsive context: what the designer states about other screen sizes. A file has no
//! breakpoints, so this holds only what was said outright: frames marked as the mobile, tablet or
//! desktop version of one design (in layer data, so the file format doesn't change), and notes in
//! the Responsive annotation category. Nothing here is worked out from names or sizes: an agent
//! must never take a guess for the designer's word.

use std::collections::BTreeSet;

use figcraft_doc::{Annotation, AnnotationCategory, Document, Node, NodeId, NodeKind};
use serde_json::{Value, json};

use crate::annotations::{MAX_CATEGORIES, MAX_PER_LAYER, categories, category_of, find_category, free_category_id, parse_label, unused_color};
use crate::cmd::CommandSpec;
use crate::params::{get, req_targets};
use crate::{Result, Session, bad, spec};

/// Layer data key: which version of a design a frame is.
pub const VIEWPORT_KEY: &str = "figcraft.viewport";
/// Layer data key: which design it is a version of.
pub const GROUP_KEY: &str = "figcraft.viewportGroup";
/// The annotation category responsive rules go in.
pub const RULES_CATEGORY: &str = "Responsive";
/// The versions a frame can be.
pub const VIEWPORTS: [&str; 3] = ["mobile", "tablet", "desktop"];
/// Longest group name (chars).
const MAX_GROUP: usize = 100;
/// Most layers looked through for rules.
const MAX_WALK: usize = 50_000;

/// Whether a layer can be marked as a version of a design: frames and sections are screens.
pub fn can_mark(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Frame | NodeKind::Section { .. })
}

/// The version `n` is marked as. Layer data can hold anything (`node.set {pluginData}`), so
/// only the three versions count.
pub fn viewport_of(n: &Node) -> Option<&str> {
    n.plugin_data.get(VIEWPORT_KEY).map(String::as_str).filter(|v| VIEWPORTS.contains(v))
}

/// The design `n` is a version of: its stored group, or its name when there is none, or the
/// stored one is empty or too long.
pub fn group_of(n: &Node) -> &str {
    n.plugin_data.get(GROUP_KEY).map(|g| g.trim()).filter(|g| !g.is_empty() && g.chars().count() <= MAX_GROUP).unwrap_or(&n.name)
}

/// A marked frame as JSON.
fn frame_json(doc: &Document, id: NodeId) -> Option<Value> {
    let n = doc.node(id)?;
    Some(json!({"id": id, "name": n.name, "viewport": viewport_of(n)?, "width": n.width, "height": n.height, "page": doc.page_of(id)}))
}

/// Every design with marked frames, narrowest first.
pub fn groups(doc: &Document) -> Vec<(String, Vec<NodeId>)> {
    let mut out: std::collections::BTreeMap<String, Vec<NodeId>> = std::collections::BTreeMap::new();
    for n in doc.nodes().filter(|n| viewport_of(n).is_some()) {
        out.entry(group_of(n).to_string()).or_default().push(n.id);
    }
    for frames in out.values_mut() {
        frames.sort_by(|a, b| doc.node(*a).map_or(0.0, |n| n.width).total_cmp(&doc.node(*b).map_or(0.0, |n| n.width)));
    }
    out.into_iter().collect()
}

fn group_json(doc: &Document, group: &str, frames: &[NodeId]) -> Value {
    json!({"group": group, "frames": frames.iter().filter_map(|f| frame_json(doc, *f)).collect::<Vec<_>>()})
}

/// Whether `a` is a responsive rule.
fn is_rule(doc: &Document, a: &Annotation) -> bool {
    category_of(doc, a).is_some_and(|c| c.name.eq_ignore_ascii_case(RULES_CATEGORY))
}

/// Counts layer `id` into `seen`: false when it is new and `max` layers are already counted.
fn take(seen: &mut BTreeSet<NodeId>, id: NodeId, max: usize) -> bool {
    seen.contains(&id) || (seen.len() < max && seen.insert(id))
}

/// `ids`, the layers above them and those under them, each once and at most `max` (sorted), and
/// whether some were left out. The walk stops at `max`: a huge subtree isn't collected first.
fn around(doc: &Document, ids: &[NodeId], max: usize) -> (Vec<NodeId>, bool) {
    let mut seen = BTreeSet::new();
    let mut walked = BTreeSet::new();
    for id in ids {
        for p in doc.ancestors(*id) {
            if !take(&mut seen, p, max) {
                return (seen.into_iter().collect(), true);
            }
        }
        let mut stack = vec![*id];
        while let Some(c) = stack.pop() {
            // A layer under an earlier target was walked with it.
            if !walked.insert(c) {
                continue;
            }
            let Some(n) = doc.node(c) else { continue };
            if !take(&mut seen, c, max) {
                return (seen.into_iter().collect(), true);
            }
            stack.extend(n.children.iter().rev());
        }
    }
    (seen.into_iter().collect(), false)
}

/// What the designer states about `ids` and the design they belong to.
pub fn context(doc: &Document, ids: &[NodeId]) -> Value {
    context_within(doc, ids, MAX_WALK)
}

/// [`context`], looking through at most `max` layers.
pub(crate) fn context_within(doc: &Document, ids: &[NodeId], max: usize) -> Value {
    let (near, cut) = around(doc, ids, max);
    let rules: Vec<Value> = near
        .iter()
        .filter_map(|i| doc.node(*i).map(|n| (i, n)))
        .flat_map(|(i, n)| n.annotations.iter().filter(|a| is_rule(doc, a)).map(move |a| json!({"id": i, "layer": n.name, "text": a.label})))
        .collect();
    let versions: Vec<Value> = groups(doc)
        .iter()
        .filter(|(_, frames)| frames.iter().any(|f| near.binary_search(f).is_ok()))
        .map(|(group, frames)| group_json(doc, group, frames))
        .collect();
    let mut out = json!({"explicit": !rules.is_empty() || !versions.is_empty(), "versions": versions, "rules": rules});
    if cut {
        // Rules on the layers left out weren't read: say so rather than imply there are none.
        out["truncated"] = json!(true);
    }
    out
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "responsive.set";
    let ids = req_targets(cmd, s, p)?;
    // `get` reads an explicit null as left out; here it means clear.
    let viewport = match p.get("viewport") {
        None => return Err(bad(cmd, "`viewport` required: mobile|tablet|desktop, or null to clear")),
        Some(Value::Null) => None,
        Some(Value::String(v)) if VIEWPORTS.contains(&v.as_str()) => Some(v.clone()),
        Some(_) => return Err(bad(cmd, "viewport: mobile|tablet|desktop, or null to clear")),
    };
    // `group`: left out keeps it, null goes back to each frame's name.
    let group = match p.get("group") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(g)) if !g.trim().is_empty() && g.trim().chars().count() <= MAX_GROUP => Some(Some(g.trim().to_string())),
        Some(_) => return Err(bad(cmd, format!("group: a name up to {MAX_GROUP} characters, or null for the frame's name"))),
    };
    let doc = s.doc_mut()?;
    for id in &ids {
        let n = doc.get(*id)?;
        if !can_mark(&n.kind) {
            return Err(bad(cmd, format!("{id} isn't a frame or section")));
        }
    }
    for id in &ids {
        let n = doc.get_mut(*id)?;
        match &viewport {
            Some(v) => {
                n.plugin_data.insert(VIEWPORT_KEY.into(), v.clone());
                match &group {
                    Some(Some(g)) => {
                        n.plugin_data.insert(GROUP_KEY.into(), g.clone());
                    }
                    Some(None) => {
                        n.plugin_data.remove(GROUP_KEY);
                    }
                    None => {}
                }
            }
            None => {
                n.plugin_data.remove(VIEWPORT_KEY);
                n.plugin_data.remove(GROUP_KEY);
            }
        }
    }
    Ok(json!({"ids": ids, "viewport": viewport}))
}

fn rule(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "responsive.rule";
    let id = req_targets(cmd, s, p)?.first().copied().ok_or_else(|| bad(cmd, "select a layer"))?;
    // Capped like `annotation.add`'s note.
    let text = parse_label(cmd, p)?.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).ok_or_else(|| bad(cmd, "`text` required"))?;
    let doc = s.doc_mut()?;
    let n = doc.get(id)?;
    // As `annotation.add`: before anything is made.
    if n.is_page() {
        return Err(bad(cmd, "pages can't be annotated"));
    }
    if n.annotations.len() >= MAX_PER_LAYER {
        return Err(bad(cmd, format!("a layer has at most {MAX_PER_LAYER} annotations")));
    }
    let mut list = categories(doc).to_vec();
    let category = match find_category(&list, RULES_CATEGORY) {
        Some(c) => c.id.clone(),
        None if list.len() >= MAX_CATEGORIES => {
            return Err(bad(cmd, format!("a file has at most {MAX_CATEGORIES} annotation categories, and none is {RULES_CATEGORY}")));
        }
        None => {
            let (cid, color) = (free_category_id(&list), unused_color(&list));
            list.push(AnnotationCategory { id: cid.clone(), name: RULES_CATEGORY.into(), color });
            doc.annotation_categories = Some(list);
            cid
        }
    };
    let n = doc.get_mut(id)?;
    n.annotations.push(Annotation { label: text, properties: Vec::new(), category: Some(category) });
    Ok(json!({"id": id, "index": n.annotations.len() - 1}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "responsive.set",
            "Mark as screen size version",
            "{ids? (frames or sections; default the selection), viewport: mobile|tablet|desktop (null: clear), group? (the design they are versions of; null or left out on a new mark: each frame's name)}  says which version of a design a frame is, in its layer data",
            true,
            set
        ),
        spec!(
            "responsive.rule",
            "Add responsive rule",
            "{id? (default the first selected layer), text (e.g. \"Sidebar becomes a drawer below 768 px\")}  a note in the Responsive annotation category, made if the file lacks it",
            true,
            rule
        ),
        spec!(
            "responsive.list",
            "List screen size versions",
            "{ids? | id? (what is stated about those layers; default every marked frame)}  only what the designer stated, nothing inferred  → {versions: [{group, frames: [{id, name, viewport, width, height, page}]}], rules?: [{id, layer, text}], explicit?, truncated? (true: too many layers to read every rule)}",
            false,
            |s, p| {
                let doc = s.doc()?;
                if get(p, "ids").is_some() || get(p, "id").is_some() {
                    let ids = req_targets("responsive.list", s, p)?;
                    return Ok(context(doc, &ids));
                }
                Ok(json!({"versions": groups(doc).iter().map(|(g, f)| group_json(doc, g, f)).collect::<Vec<_>>()}))
            }
        ),
    ]
}
