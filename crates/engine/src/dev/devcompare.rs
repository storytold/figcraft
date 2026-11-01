//! Dev Mode "Compare changes": what changed in a ready-for-dev frame or section since it was
//! marked, or in any frame or section since a saved version. The document keeps a snapshot of a
//! ready-for-dev layer from just before its first change ([`figcraft_doc::handoff`]); this compares
//! it (or the layer in the version) with the layer as it is now, layer by layer
//! (matched by id): layers added and removed, and for the others their properties formatted the
//! way Inspect shows them (`width 120 → 136`, `fill #FF0000 → #E03030`).

use std::collections::BTreeSet;

use figcraft_doc::{DevStatus, Document, MAX_SNAPSHOT_NODES, Node, NodeId, NodeKind};
use serde::Serialize;

use crate::devinspect::{Prop, Section, properties};

/// Most changed layers listed (the rest is counted as truncated).
pub const MAX_CHANGES: usize = 2_000;
/// Deepest nesting walked.
const MAX_DEPTH: usize = 256;
/// Longest text content shown in a change (chars).
const MAX_TEXT: usize = 60;

/// One property that changed, as Inspect formats it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PropChange {
    /// The group it belongs to ("Layer", "Layout", "Fills"…).
    pub section: &'static str,
    /// What changed, lower case ("width", "fill", "font size"…).
    pub property: String,
    pub before: String,
    pub after: String,
    /// The whole change as one line: `width 120 → 136`.
    pub text: String,
}

impl PropChange {
    pub fn new(section: &'static str, property: String, before: String, after: String) -> Self {
        let text = format!("{property} {before} → {after}");
        Self { section, property, before, after, text }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
}

impl ChangeKind {
    pub fn label(self) -> &'static str {
        match self {
            ChangeKind::Added => "Added",
            ChangeKind::Removed => "Removed",
            ChangeKind::Changed => "Changed",
        }
    }
}

/// A layer that was added, removed or changed.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerChange {
    pub id: NodeId,
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: &'static str,
    pub change: ChangeKind,
    /// How deep under the compared layer it is (0: the layer itself).
    pub depth: usize,
    /// Added or removed: how many layers that is (the layer and everything in it).
    pub layers: usize,
    /// Changed: what changed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<PropChange>,
}

/// The changes to a ready-for-dev layer since it was marked.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub id: NodeId,
    pub name: String,
    /// It changed since it was marked (shown as "Changed").
    pub changed: bool,
    /// What it looked like before is known (files from before snapshots were kept, and layers too
    /// large to snapshot, have none).
    pub snapshot: bool,
    pub changes: Vec<LayerChange>,
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    /// More than [`MAX_CHANGES`] layers changed: the list stops there.
    pub truncated: bool,
}

/// Nothing compared yet for layer `id` of `doc` (`snapshot` false: the before isn't known).
pub fn empty(doc: &Document, id: NodeId) -> Comparison {
    Comparison {
        id,
        name: doc.node(id).map(|n| n.name.clone()).unwrap_or_default(),
        changed: false,
        snapshot: false,
        changes: Vec::new(),
        added: 0,
        removed: 0,
        modified: 0,
        truncated: false,
    }
}

/// The changes to layer `id` since it was marked ready for dev (empty when it hasn't changed).
pub fn compare(doc: &Document, id: NodeId) -> Comparison {
    let mut out = empty(doc, id);
    out.changed = doc.node(id).is_some_and(|n| n.dev_status == DevStatus::ReadyForDev && n.dev_changed);
    if !out.changed {
        return out;
    }
    let Some(before) = doc.with_dev_snapshot(id) else { return out };
    out.snapshot = true;
    let mut w = Walk { before: &before, now: doc, now_ids: bounded_set(doc, id), before_ids: bounded_set(&before, id), out: &mut out };
    w.layer(id, 0);
    out
}

/// The changes to layer `id` of `now` since `before` (a saved version of the same file: layers
/// match by id). A layer the version doesn't have was added as a whole.
pub fn compare_with(before: &Document, now: &Document, id: NodeId) -> Comparison {
    let mut out = empty(now, id);
    out.snapshot = true;
    let mut w = Walk { before, now, now_ids: bounded_set(now, id), before_ids: bounded_set(before, id), out: &mut out };
    if before.contains(id) {
        w.layer(id, 0);
    } else {
        w.whole(now, id, 0, ChangeKind::Added);
    }
    out.changed = !out.changes.is_empty();
    out
}

/// The before and after documents of layer `id` (the snapshot's copy and the document), for
/// rendering the two side by side.
pub fn before_doc(doc: &Document, id: NodeId) -> Option<Document> {
    doc.node(id).filter(|n| n.dev_status == DevStatus::ReadyForDev && n.dev_changed)?;
    doc.with_dev_snapshot(id)
}

/// Every layer that changed between two versions of a whole document, page by page (pages added
/// or removed as a whole), for reviewing a branch.
pub fn compare_docs(before: &Document, now: &Document) -> Comparison {
    let mut out = Comparison { name: now.name.clone(), snapshot: true, ..empty(now, NodeId(0)) };
    let mut w = Walk { before, now, now_ids: now.ids().collect(), before_ids: before.ids().collect(), out: &mut out };
    for p in &now.pages {
        if before.contains(*p) {
            w.layer(*p, 0);
        } else {
            w.whole(now, *p, 0, ChangeKind::Added);
        }
    }
    for p in before.pages.iter().filter(|p| !now.contains(**p)) {
        w.whole(before, *p, 0, ChangeKind::Removed);
    }
    out.changed = !out.changes.is_empty();
    out
}

fn bounded_set(doc: &Document, id: NodeId) -> BTreeSet<NodeId> {
    doc.subtree(id).into_iter().take(MAX_SNAPSHOT_NODES * 2).collect()
}

struct Walk<'a> {
    before: &'a Document,
    now: &'a Document,
    now_ids: BTreeSet<NodeId>,
    before_ids: BTreeSet<NodeId>,
    out: &'a mut Comparison,
}

impl Walk<'_> {
    fn push(&mut self, c: LayerChange) {
        if self.out.changes.len() >= MAX_CHANGES {
            self.out.truncated = true;
            return;
        }
        match c.change {
            ChangeKind::Added => self.out.added += 1,
            ChangeKind::Removed => self.out.removed += 1,
            ChangeKind::Changed => self.out.modified += 1,
        }
        self.out.changes.push(c);
    }

    fn whole(&mut self, doc: &Document, id: NodeId, depth: usize, change: ChangeKind) {
        let Some(n) = doc.node(id) else { return };
        let layers = doc.subtree(id).len().min(MAX_SNAPSHOT_NODES * 2);
        self.push(LayerChange { id, name: n.name.clone(), type_name: n.kind.type_name(), change, depth, layers, properties: Vec::new() });
    }

    /// Layer `id` (in both versions) and what's under it: its own changes, then its children as they
    /// are now (added ones as a whole), then the children it lost (removed as a whole).
    fn layer(&mut self, id: NodeId, depth: usize) {
        if depth > MAX_DEPTH || self.out.truncated {
            return;
        }
        let Some(now) = self.now.node(id) else { return };
        // A layer shared by both versions under a shared parent didn't change (nor its place).
        let shared = |n: NodeId| self.before.node_arc(n).zip(self.now.node_arc(n)).is_some_and(|(x, y)| std::sync::Arc::ptr_eq(x, y));
        let props = if shared(id) && now.parent.is_none_or(shared) { Vec::new() } else { layer_changes(self.before, self.now, id) };
        if !props.is_empty() {
            self.push(LayerChange {
                id,
                name: now.name.clone(),
                type_name: now.kind.type_name(),
                change: ChangeKind::Changed,
                depth,
                layers: 1,
                properties: props,
            });
        }
        for c in now.children.iter().rev() {
            if self.before_ids.contains(c) {
                self.layer(*c, depth + 1);
            } else {
                self.whole(self.now, *c, depth + 1, ChangeKind::Added);
            }
        }
        let lost: Vec<NodeId> =
            self.before.node(id).map(|b| b.children.iter().rev().copied().filter(|c| !self.now_ids.contains(c)).collect()).unwrap_or_default();
        for c in lost {
            self.whole(self.before, c, depth + 1, ChangeKind::Removed);
        }
    }
}

/// What changed on layer `id` between `before` and `now` (both must hold it).
pub fn layer_changes(before: &Document, now: &Document, id: NodeId) -> Vec<PropChange> {
    let (Some(b), Some(a)) = (before.node(id), now.node(id)) else { return Vec::new() };
    let mut out = Vec::new();
    let mut put = |section: &'static str, property: &str, before: String, after: String| {
        if before != after {
            out.push(PropChange::new(section, property.to_string(), before, after));
        }
    };
    let parent_name = |d: &Document, n: &Node| n.parent.and_then(|p| d.node(p)).map(|p| p.name.clone()).unwrap_or_default();
    if b.parent != a.parent {
        put("Layer", "parent", parent_name(before, b), parent_name(now, a));
    } else if let Some((ib, ia)) = order(before, now, id) {
        put("Layer", "order", ib.to_string(), ia.to_string());
    }
    // Unchanged layers (shared with the snapshot, or equal) only move with their parent.
    let same = before.node_arc(id).zip(now.node_arc(id)).is_some_and(|(x, y)| std::sync::Arc::ptr_eq(x, y)) || b.same_design(a);
    if same {
        return out;
    }
    put("Layer", "name", b.name.clone(), a.name.clone());
    let shown = |n: &Node| if n.visible { "visible" } else { "hidden" }.to_string();
    put("Layer", "visibility", shown(b), shown(a));
    if b.blend != a.blend {
        put("Layer", "blend", b.blend.label().to_string(), a.blend.label().to_string());
    }
    if a.kind.is_frame_like() && b.kind.is_frame_like() {
        let yes = |v: bool| if v { "on" } else { "off" }.to_string();
        put("Layer", "clip content", yes(b.clip), yes(a.clip));
    }
    if let (NodeKind::Text(tb), NodeKind::Text(ta)) = (&b.kind, &a.kind) {
        put("Layer", "text", quoted(&tb.characters), quoted(&ta.characters));
    }
    if let (NodeKind::Instance { component: cb, props: pb }, NodeKind::Instance { component: ca, props: pa }) = (&b.kind, &a.kind) {
        let main = |d: &Document, c: &NodeId| d.node(*c).map(|n| n.name.clone()).unwrap_or_else(|| c.to_string());
        put("Layer", "component", main(before, cb), main(now, ca));
        let keys: BTreeSet<&String> = pb.keys().chain(pa.keys()).collect();
        for k in keys {
            let v = |m: &std::collections::BTreeMap<String, serde_json::Value>| m.get(k).map(value_text).unwrap_or_else(|| "default".into());
            put("Layer", k.split('#').next().unwrap_or(k), v(pb), v(pa));
        }
    }
    if b.kind.type_name() != a.kind.type_name() {
        put("Layer", "type", b.kind.type_name().to_string(), a.kind.type_name().to_string());
    }
    for c in section_changes(&properties(before, id), &properties(now, id)) {
        out.push(c);
    }
    out
}

/// Layer `id`'s place (1 = top of the layers panel) among the siblings it had in both versions,
/// when that changed.
fn order(before: &Document, now: &Document, id: NodeId) -> Option<(usize, usize)> {
    let parent = now.node(id)?.parent?;
    let (pb, pa) = (before.node(parent)?, now.node(parent)?);
    let common_b: Vec<NodeId> = pb.children.iter().rev().copied().filter(|c| pa.children.contains(c)).collect();
    let common_a: Vec<NodeId> = pa.children.iter().rev().copied().filter(|c| pb.children.contains(c)).collect();
    let (ib, ia) = (common_b.iter().position(|c| *c == id)?, common_a.iter().position(|c| *c == id)?);
    (ib != ia).then_some((ib + 1, ia + 1))
}

fn quoted(s: &str) -> String {
    let mut t: String = s.chars().take(MAX_TEXT).collect();
    if s.chars().count() > MAX_TEXT {
        t.push('…');
    }
    format!("“{t}”")
}

fn value_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The sections whose rows form a list (one row per paint or effect).
const LISTS: [(&str, &str); 3] = [("Fills", "fill"), ("Strokes", "stroke"), ("Effects", "effect")];

/// The changes between two layers' inspected properties.
fn section_changes(before: &[Section], after: &[Section]) -> Vec<PropChange> {
    let mut titles: Vec<&'static str> = after.iter().map(|s| s.title).collect();
    for s in before {
        if !titles.contains(&s.title) {
            titles.push(s.title);
        }
    }
    let rows = |list: &[Section], title: &str| -> Vec<Prop> { list.iter().find(|s| s.title == title).map(|s| s.rows.clone()).unwrap_or_default() };
    let mut out = Vec::new();
    for title in titles {
        let (b, a) = (rows(before, title), rows(after, title));
        match LISTS.iter().find(|(t, _)| *t == title) {
            Some((_, base)) => {
                // A stroke's weight is a row of its own, not a paint.
                let (bw, b): (Vec<Prop>, Vec<Prop>) = b.into_iter().partition(|r| r.label == "Weight");
                let (aw, a): (Vec<Prop>, Vec<Prop>) = a.into_iter().partition(|r| r.label == "Weight");
                list_changes(title, base, &b, &a, &mut out);
                keyed_changes(title, &bw, &aw, &mut out);
            }
            None => keyed_changes(title, &b, &a, &mut out),
        }
    }
    out
}

/// Rows paired by position: `fill #FF0000 → #E03030`, `fill 2 none → #FFFFFF`.
fn list_changes(section: &'static str, base: &str, before: &[Prop], after: &[Prop], out: &mut Vec<PropChange>) {
    let n = before.len().max(after.len());
    for i in 0..n {
        let (b, a) = (before.get(i), after.get(i));
        // Same kind of effect: name it ("drop shadow"); paints are named by their list.
        let kind = match (b, a) {
            (Some(x), Some(y)) if base == "effect" && x.label == y.label => x.label.to_lowercase(),
            (Some(x), None) | (None, Some(x)) if base == "effect" => x.label.to_lowercase(),
            _ => base.to_string(),
        };
        let property = if n > 1 { format!("{kind} {}", i + 1) } else { kind };
        let text = |p: Option<&Prop>, other: Option<&Prop>| match p {
            Some(p) => {
                let v = if p.label == "Solid" || base == "effect" && other.is_some_and(|o| o.label == p.label) {
                    p.value.clone()
                } else {
                    format!("{} {}", p.label, p.value)
                };
                with_token(v, p, other)
            }
            None => "none".to_string(),
        };
        let (tb, ta) = (text(b, a), text(a, b));
        if tb != ta {
            out.push(PropChange::new(section, property, tb, ta));
        }
    }
}

/// Rows paired by label: `width 120 → 136`.
fn keyed_changes(section: &'static str, before: &[Prop], after: &[Prop], out: &mut Vec<PropChange>) {
    let mut labels: Vec<&str> = after.iter().map(|r| r.label.as_str()).collect();
    for r in before {
        if !labels.contains(&r.label.as_str()) {
            labels.push(&r.label);
        }
    }
    for label in labels {
        // Text content is compared as the layer's text.
        if label == "Content" {
            continue;
        }
        let (b, a) = (before.iter().find(|r| r.label == label), after.iter().find(|r| r.label == label));
        let text = |p: Option<&Prop>, other: Option<&Prop>| match p {
            Some(p) => with_token(p.value.clone(), p, other),
            None => missing(section, label).to_string(),
        };
        let (tb, ta) = (text(b, a), text(a, b));
        if tb != ta {
            out.push(PropChange::new(section, property_name(section, label), tb, ta));
        }
    }
}

/// A row's value, with the style or variable it comes from when that differs from the other side's.
fn with_token(value: String, p: &Prop, other: Option<&Prop>) -> String {
    match &p.token {
        Some(t) if other.is_none_or(|o| o.token.as_ref() != Some(t)) => format!("{value} ({t})"),
        _ => value,
    }
}

/// What a row Inspect leaves out means.
fn missing(section: &str, label: &str) -> &'static str {
    match (section, label) {
        (_, "Rotation") => "0°",
        (_, "Opacity") => "100%",
        (_, "Radius") | (_, "Paragraph spacing") => "0",
        _ => "none",
    }
}

/// A row's property name in a change: Inspect's label, lower case, made unambiguous.
fn property_name(section: &str, label: &str) -> String {
    let name = match (section, label) {
        ("Layout", "Radius") => "corner radius",
        ("Auto layout", "Direction") => "auto layout",
        ("Auto layout", "Align") => "alignment",
        ("Typography", "Style") => "font style",
        ("Typography", "Size") => "font size",
        ("Typography", "Align") => "text align",
        ("Strokes", "Weight") => "stroke weight",
        _ => return label.to_lowercase(),
    };
    name.to_string()
}
