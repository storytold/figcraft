//! Decoded `.fig` message → REST API file JSON, which [`crate::rest`] then imports.
//!
//! The message lists node changes; each names its id (`guid`), its parent and its position among
//! the siblings (`parentIndex`). Nodes are read by field name, so files written with newer
//! schemas still import what is understood. Instances carry only their overrides: their layers
//! are expanded here from the main component (overrides and derived layout matched by id path,
//! component property values applied) so the import looks as saved.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use figcraft_doc::VectorNetwork;
use figcraft_doc::geom::{Affine, Point};
use figcraft_doc::proto::{MAX_ACTION_DEPTH, MAX_EXPR_DEPTH};
use serde_json::{Map, Value, json};

use super::kiwi::Kv;
use super::network;

type Obj = Map<String, Value>;

/// Deepest nesting followed.
const MAX_DEPTH: usize = 256;
/// Most path commands read from one geometry blob.
const MAX_COMMANDS: usize = 200_000;
const MAX_GRID_TRACKS: usize = 1_000;
const MAX_GRID_TRACK_VALUE: f64 = 1e6;
/// Id of "no style".
const NO_STYLE: &str = "4294967295:4294967295";
/// Longest asset `key` or `version` kept for resolving references given as assets.
const MAX_ASSET_PART: usize = 256;

/// The REST API file JSON for message `msg`, with the warnings about what was skipped and the
/// vector networks its layers name by `vectorNetworkRef` (for [`crate::rest::import_with_networks`]).
pub(super) fn to_rest(msg: &Kv, name: &str, max_nodes: usize) -> (Value, Vec<String>, Vec<Option<VectorNetwork>>) {
    let mut m = Mapper::new(msg, max_nodes);
    let doc = m.document();
    // Large values are moved in, never passed through `json!`, which serialises its arguments:
    // a deep copy of the whole tree, briefly holding it twice.
    let mut out = json!({"name": name});
    out["document"] = doc;
    if !m.library_contexts.is_empty() {
        out[crate::rest::FIG_LIBRARY_CONTEXTS_FIELD] = Value::Array(std::mem::take(&mut m.library_contexts));
    }
    if !m.styles.is_empty() {
        out["styles"] = Value::Object(std::mem::take(&mut m.styles));
    }
    if !m.components.is_empty() {
        out["components"] = Value::Object(std::mem::take(&mut m.components));
    }
    if !m.sets.is_empty() {
        out["componentSets"] = Value::Object(std::mem::take(&mut m.sets));
    }
    if let Some(vars) = m.variables() {
        out["meta"] = vars;
    }
    let styles = m.style_definitions();
    if !styles.is_empty() {
        out["styleDefinitions"] = Value::Array(styles);
    }
    (out, m.warnings, m.networks)
}

#[cfg(test)]
#[path = "map_image_filter_tests.rs"]
mod image_filter_tests;

/// REST importer warnings reworded for a `.fig` import (`None`: drop it).
pub(super) fn adapt_warning(w: String) -> Option<String> {
    if w.contains("geometry=paths") {
        return None;
    }
    if w.contains("without image data") {
        let count = w.split_whitespace().next().unwrap_or("Some");
        return Some(format!("{count} image fill(s) whose image isn't in the file show a grey placeholder"));
    }
    Some(w.replace("the variables JSON", "the file").replace("the file JSON", "the file"))
}

// ---- small readers ----

fn num(n: &Kv, k: &str) -> Option<f64> {
    n.get(k).and_then(Kv::f64)
}

fn filter_number(value: &Kv) -> Option<f64> {
    let number = match value {
        Kv::Float(value) => f64::from(*value),
        Kv::Int(value) => *value as f64,
        Kv::Uint(value) => *value as f64,
        _ => return None,
    };
    number.is_finite().then_some(number)
}

fn guid(v: &Kv) -> Option<String> {
    Some(format!("{}:{}", num(v, "sessionID")? as u64, num(v, "localID")? as u64))
}

/// An override row names a layer of the main by its current GUID or by the `overrideKey` the
/// layer had when the row was written: the key becomes the current GUID, unless the layer is
/// current already or the key is ambiguous.
fn normalize_override_segment(path: &mut [String], index: usize, scope: &OverrideScope) {
    let Some(segment) = path.get(index) else { return };
    if scope.current.contains(segment) {
        return;
    }
    if let Some(Some(current)) = scope.aliases.get(segment) {
        path[index] = current.clone();
    }
}

#[cfg(test)]
#[path = "map_connector_snapshot_budget_tests.rs"]
mod connector_snapshot_budget_tests;

/// A `GUID`-holding reference (`StyleId`, `VariableID`, …): its `guid`.
fn ref_guid(v: &Kv) -> Option<String> {
    v.get("guid").and_then(guid).filter(|g| g != NO_STYLE)
}

/// The style a layer links in `key`, or in the older `legacy` field when `key` names none. An
/// explicit NO_STYLE in `key` is a detach and wins over the legacy field.
fn style_link(v: &View, key: &str, legacy: &str) -> Option<String> {
    match v.get(key).and_then(|r| r.get("guid")).and_then(guid) {
        Some(id) => Some(id).filter(|g| g != NO_STYLE),
        None => v.get(legacy).and_then(guid).filter(|g| g != NO_STYLE),
    }
}

fn color(c: &Kv) -> Value {
    let ch = |k: &str| num(c, k).unwrap_or(0.0);
    json!({"r": ch("r"), "g": ch("g"), "b": ch("b"), "a": num(c, "a").unwrap_or(1.0)})
}

fn vec2(v: &Kv) -> Option<Value> {
    Some(json!({"x": num(v, "x")?, "y": num(v, "y")?}))
}

fn size(v: &Kv) -> Option<(f64, f64)> {
    Some((num(v, "x")?, num(v, "y")?))
}

fn matrix(m: &Kv) -> Option<Affine> {
    let c = |k: &str| num(m, k);
    let a = Affine::new([c("m00")?, c("m10")?, c("m01")?, c("m11")?, c("m02")?, c("m12")?]);
    a.as_coeffs().iter().all(|x| x.is_finite()).then_some(a)
}

fn rows(a: Affine) -> Value {
    let [a, b, c, d, e, f] = a.as_coeffs();
    json!([[a, c, e], [b, d, f]])
}

fn copy_num(o: &mut Obj, n: &View, from: &str, to: &str) {
    if let Some(x) = n.num(from) {
        o.insert(to.into(), json!(x));
    }
}

fn copy_str(o: &mut Obj, n: &View, from: &str, to: &str) {
    if let Some(s) = n.str(from) {
        o.insert(to.into(), json!(s));
    }
}

/// The value from the last override layer holding a valid one for an optional text setting; invalid instance rows fall through.
fn text_num(v: &View, key: &str) -> Option<f64> {
    v.layers.iter().rev().find_map(|layer| {
        layer.get(key).and_then(|value| match value {
            Kv::Bool(_) => None,
            _ => value.f64(),
        })
    })
}

fn text_flag(v: &View, key: &str) -> Option<bool> {
    v.layers.iter().rev().find_map(|layer| layer.get(key).and_then(Kv::bool))
}

fn insert_text_options(style: &mut Obj, v: &View) {
    if let Some(value) = text_num(v, "listSpacing") {
        style.insert("listSpacing".into(), json!(value));
    }
    for k in ["hangingList", "hangingPunctuation"] {
        if let Some(value) = text_flag(v, k) {
            style.insert(k.into(), json!(value));
        }
    }
    if let Some(value) = leading_trim(v) {
        style.insert("leadingTrim".into(), json!(value));
    }
}

fn leading_trim<'a>(v: &View<'a>) -> Option<&'a str> {
    v.layers.iter().rev().find_map(|layer| match layer.get("leadingTrim").and_then(Kv::str) {
        Some(value @ ("NONE" | "CAP_HEIGHT")) => Some(value),
        _ => None,
    })
}

/// A node seen through override layers: the last layer with a field wins.
#[derive(Clone)]
struct View<'a> {
    layers: Vec<&'a Kv<'a>>,
    /// Size and transform worked out from the constraints, over the layers' own.
    fitted: Option<((f64, f64), Affine)>,
}

impl<'a> View<'a> {
    fn of(n: &'a Kv<'a>) -> Self {
        Self { layers: vec![n], fitted: None }
    }

    fn size(&self) -> Option<(f64, f64)> {
        self.fitted.map(|f| f.0).or_else(|| self.get("size").and_then(size))
    }

    fn transform(&self) -> Option<Affine> {
        self.fitted.map(|f| f.1).or_else(|| self.get("transform").and_then(matrix))
    }

    fn with(&self, top: impl IntoIterator<Item = &'a Kv<'a>>) -> Self {
        let mut v = self.clone();
        v.layers.extend(top);
        v
    }

    /// This view with `style` just above the node itself: the style beats the node's own copy of
    /// its fields (stale when the style changed since), and override rows beat both.
    fn styled(&self, style: Option<&'a Kv<'a>>) -> Self {
        let mut v = self.clone();
        if let Some(s) = style {
            v.layers.insert(v.layers.len().min(1), s);
        }
        v
    }

    fn get(&self, k: &str) -> Option<&'a Kv<'a>> {
        self.layers.iter().rev().find_map(|l| l.get(k))
    }

    fn num(&self, k: &str) -> Option<f64> {
        self.get(k).and_then(Kv::f64)
    }

    fn str(&self, k: &str) -> Option<&'a str> {
        self.get(k).and_then(Kv::str)
    }

    fn flag(&self, k: &str) -> Option<bool> {
        self.get(k).and_then(Kv::bool)
    }

    fn list(&self, k: &str) -> &'a [Kv<'a>] {
        self.get(k).map(Kv::list).unwrap_or_default()
    }
}

/// How the layers of a resized container follow it when the file gives no derived layout for
/// them: a frame's by their constraints, from its main's size to its own; a group's scale with it.
#[derive(Clone, Copy)]
enum Fit {
    Frame { from: (f64, f64), to: (f64, f64) },
    Scale(f64, f64),
}

impl Fit {
    fn frame(from: Option<(f64, f64)>, to: Option<(f64, f64)>) -> Option<Self> {
        let (from, to) = (from?, to?);
        ((from.0 - to.0).abs() > 1e-9 || (from.1 - to.1).abs() > 1e-9).then_some(Fit::Frame { from, to })
    }

    fn scale(from: Option<(f64, f64)>, to: Option<(f64, f64)>) -> Option<Self> {
        let (from, to) = (from?, to?);
        let k = |a: f64, b: f64| if a > 1e-9 { b / a } else { 1.0 };
        let (kx, ky) = (k(from.0, to.0), k(from.1, to.1));
        ((kx - 1.0).abs() > 1e-9 || (ky - 1.0).abs() > 1e-9).then_some(Fit::Scale(kx, ky))
    }

    /// The size and transform (in the parent's coordinates) of a `w` × `h` layer at `t` with
    /// constraints `h_con`, `v_con`. Like the editor's constraint solver, a layer keeps its
    /// rotation and flip and stretches along its own axes.
    fn apply(self, w: f64, h: f64, t: Affine, h_con: Option<&str>, v_con: Option<&str>) -> ((f64, f64), Affine) {
        let [a, b, c, d, _, _] = t.as_coeffs();
        let lin = Affine::new([a, b, c, d, 0.0, 0.0]);
        let bb = figcraft_doc::geom::transform_rect_bbox(t, figcraft_doc::geom::Rect::new(0.0, 0.0, w, h));
        // Where the new box goes: its top-left (constraints) or its centre (a group's scale).
        let (kx, ky, x0, y0, centred) = match self {
            Fit::Frame { from, to } => {
                let axis = |con: Option<&str>, lo: f64, hi: f64, old: f64, new: f64| match con {
                    Some("MAX" | "FIXED_MAX") => (lo + new - old, hi + new - old),
                    Some("STRETCH") => (lo, hi + new - old),
                    Some("CENTER") => (lo + (new - old) / 2.0, hi + (new - old) / 2.0),
                    Some("SCALE") if old > 1e-9 => (lo * new / old, hi * new / old),
                    _ => (lo, hi),
                };
                let (x0, x1) = axis(h_con, bb.x0, bb.x1, from.0, to.0);
                let (y0, y1) = axis(v_con, bb.y0, bb.y1, from.1, to.1);
                let k = |new: f64, old: f64| if old > 1e-9 { (new / old).max(0.0) } else { 1.0 };
                (k(x1 - x0, bb.width()), k(y1 - y0, bb.height()), x0, y0, false)
            }
            Fit::Scale(kx, ky) => (kx, ky, bb.center().x * kx, bb.center().y * ky, true),
        };
        let along = |x: f64, y: f64, fallback: f64| {
            let len = x.hypot(y);
            let k = (kx * x).hypot(ky * y) / len;
            if len > 1e-12 && k.is_finite() { k } else { fallback }
        };
        let size = (w * along(a, b, kx), h * along(c, d, ky));
        let nb = figcraft_doc::geom::transform_rect_bbox(lin, figcraft_doc::geom::Rect::new(0.0, 0.0, size.0, size.1));
        let at = if centred { nb.center() } else { figcraft_doc::geom::Point::new(nb.x0, nb.y0) };
        (size, Affine::translate((x0 - at.x, y0 - at.y)) * lin)
    }
}

/// Overrides and derived layout by id path.
type Overrides<'a> = BTreeMap<Vec<String>, Vec<OverrideRow<'a>>>;

/// Where an override entry comes from.
#[derive(Clone, Copy, PartialEq)]
enum Entry {
    Override,
    Derived,
    /// Derived layout of a nested instance for a size it no longer has: its size and place no
    /// longer hold, the rest (resolved colours, text) still does.
    Stale,
}

/// One override or derived-layout row addressed to a layer.
#[derive(Clone)]
struct OverrideRow<'a> {
    row: &'a Kv<'a>,
    entry: Entry,
    /// How many mains are being expanded around the instance that wrote the row: the rows of
    /// the enclosing (outer) instances win over those of the inner ones.
    source_depth: usize,
    /// Position among the rows of the instance that wrote it; later rows win.
    ordinal: usize,
}

/// The layers of a main by current GUID, and the old `overrideKey` of each that has one
/// (`None` when two layers claim the same key).
struct OverrideScope {
    current: BTreeSet<String>,
    aliases: BTreeMap<String, Option<String>>,
}

/// Where an instance's expanded layers are: the overrides and derived layout of the outermost
/// instance (by id path), the path of the instance being expanded, and the property values.
#[derive(Clone)]
struct Inst<'a> {
    root: String,
    path: Vec<String>,
    overrides: Rc<Overrides<'a>>,
    props: Rc<BTreeMap<String, &'a Kv<'a>>>,
    /// Slot property GUID → private slot-content node GUID.
    slot_contents: Rc<BTreeMap<String, String>>,
    /// Layers imported from a slot's independent content subtree.
    slot_content: bool,
    /// Mains being expanded (an instance inside its own main stops there).
    symbols: Vec<String>,
}

struct Mapper<'a> {
    nodes: BTreeMap<String, &'a Kv<'a>>,
    kids: BTreeMap<String, Vec<(&'a str, String)>>,
    blobs: Vec<&'a [u8]>,
    /// Component property definitions: id → (REST name "Name#id", type).
    prop_defs: BTreeMap<String, (String, &'a str)>,
    /// File nodes by asset `key` and `version`: references can name a collection, variable or
    /// style as the asset instead of the guid, and the file carries the resources to resolve
    /// them (the collection node itself says the same `key`).
    assets: BTreeMap<String, BTreeMap<String, String>>,
    styles: Obj,
    /// Descriptions of components and component sets (REST `components` / `componentSets`).
    components: Obj,
    sets: Obj,
    /// Layers and keys of each main, built when an instance of it first needs them.
    override_scopes: BTreeMap<String, Rc<OverrideScope>>,
    /// Entries held by `override_scopes` (bounded by `max_nodes` together).
    override_scope_entries: usize,
    /// Component-bearing branches of the file's internal canvases (cached library definitions).
    library_contexts: Vec<Value>,
    /// Layers visited while scanning internal canvases, across all of them.
    library_scan_count: usize,
    warnings: Vec<String>,
    count: usize,
    /// Most layers produced or scanned, instance expansion included; the one place the cap comes
    /// from (see [`super::layer_budget`]).
    max_nodes: usize,
    /// Vector networks, each built once: the layers that show one (every instance of a main
    /// holding it) name it by `vectorNetworkRef`. Kept out of the JSON, where a large file's
    /// networks took gigabytes.
    networks: Vec<Option<VectorNetwork>>,
    /// Index in `networks` by what a network is made from: its blob, its scale and its style table.
    network_ids: HashMap<(usize, u64, u64, usize), usize>,
}

impl<'a> Mapper<'a> {
    fn new(msg: &'a Kv<'a>, max_nodes: usize) -> Self {
        let mut nodes = BTreeMap::new();
        let mut kids: BTreeMap<String, Vec<(&'a str, String)>> = BTreeMap::new();
        let mut prop_defs = BTreeMap::new();
        let changes = msg.get("nodeChanges").map(Kv::list).unwrap_or_default();
        // The last change of a GUID wins; a REMOVED change deletes what came before it.
        for n in changes {
            let Some(id) = n.get("guid").and_then(guid) else { continue };
            if n.get("phase").and_then(Kv::str) == Some("REMOVED") {
                nodes.remove(&id);
            } else {
                nodes.insert(id, n);
            }
        }
        for n in changes {
            let Some(id) = n.get("guid").and_then(guid) else { continue };
            if !nodes.get(&id).is_some_and(|latest| std::ptr::eq(*latest, n)) {
                continue;
            }
            if let Some(p) = n.get("parentIndex")
                && let Some(parent) = p.get("guid").and_then(guid)
            {
                kids.entry(parent).or_default().push((p.get("position").and_then(Kv::str).unwrap_or(""), id.clone()));
            }
            for d in n.get("componentPropDefs").map(Kv::list).unwrap_or_default() {
                if let (Some(did), Some(name), Some(ty)) =
                    (d.get("id").and_then(guid), d.get("name").and_then(Kv::str), d.get("type").and_then(Kv::str))
                {
                    prop_defs.insert(did.clone(), (format!("{name}#{did}"), ty));
                }
            }
        }
        for list in kids.values_mut() {
            list.sort();
            list.dedup_by(|a, b| a.1 == b.1);
        }
        let blobs =
            msg.get("blobs").map(Kv::list).unwrap_or_default().iter().map(|b| b.get("bytes").and_then(Kv::bytes).unwrap_or_default()).collect();
        let mut assets: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for n in msg.get("nodeChanges").map(Kv::list).unwrap_or_default() {
            let (Some(key), Some(id)) = (n.get("key").and_then(Kv::str), n.get("guid").and_then(guid)) else { continue };
            if key.is_empty() || key.len() > MAX_ASSET_PART {
                continue;
            }
            let version = n.get("version").and_then(Kv::str).filter(|v| !v.is_empty() && v.len() <= MAX_ASSET_PART).unwrap_or("");
            assets.entry(key.to_string()).or_default().entry(version.to_string()).or_insert(id);
        }
        Self {
            nodes,
            kids,
            blobs,
            prop_defs,
            assets,
            styles: Obj::new(),
            components: Obj::new(),
            sets: Obj::new(),
            override_scopes: BTreeMap::new(),
            override_scope_entries: 0,
            library_contexts: Vec::new(),
            library_scan_count: 0,
            warnings: Vec::new(),
            count: 0,
            max_nodes,
            networks: Vec::new(),
            network_ids: HashMap::new(),
        }
    }

    /// The id of the resource a `VariableSetID` (or `VariableID`, `StyleId`) names: its `guid`,
    /// else the file node its asset `key` and `version` names (a collection from a library is
    /// referenced as the asset, and the file's own node for it says the same `key`). `None` when
    /// the reference has neither or the asset names nothing (or ambiguously) in the file.
    /// Original code for this importer: the field names are those of the schema each file embeds.
    fn resource_id(&self, v: &Kv) -> Option<String> {
        if let Some(g) = v.get("guid").and_then(guid).filter(|g| *g != NO_STYLE) {
            return Some(g);
        }
        let asset = v.get("assetRef")?;
        let key = asset.get("key").and_then(Kv::str)?;
        let versions = self.assets.get(key)?;
        let version = asset.get("version").and_then(Kv::str);
        match version {
            Some(v) => versions.get(v),
            // Without a version: an unversioned record, else the file's only one.
            None => versions.get("").or_else(|| (versions.len() == 1).then(|| versions.values().next()).flatten()),
        }
        .cloned()
    }

    fn warn(&mut self, w: impl Into<String>) {
        let w = w.into();
        // Capped like the REST importer's list: some warnings name fields of the file (image filters).
        if self.warnings.len() < crate::rest::MAX_WARNINGS && !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }

    fn children(&self, id: &str) -> Vec<String> {
        self.kids.get(id).map(|k| k.iter().map(|(_, c)| c.clone()).collect()).unwrap_or_default()
    }

    /// The layers of main `main` by current GUID and by `overrideKey`. An instance nested in the
    /// main is not entered: its layers belong to another main.
    fn override_scope(&mut self, main: &str) -> Rc<OverrideScope> {
        if let Some(scope) = self.override_scopes.get(main) {
            return Rc::clone(scope);
        }
        let mut current = BTreeSet::new();
        let mut aliases: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut visited = BTreeSet::new();
        let mut pending: Vec<(String, usize)> = self.children(main).into_iter().map(|id| (id, 1)).collect();
        let mut complete = true;
        while let Some((id, depth)) = pending.pop() {
            if depth > MAX_DEPTH || visited.len() >= self.max_nodes {
                complete = false;
                break;
            }
            if !visited.insert(id.clone()) {
                continue;
            }
            let Some(node) = self.nodes.get(&id) else { continue };
            current.insert(id.clone());
            if let Some(alias) = node.get("overrideKey").and_then(guid) {
                let target = aliases.entry(alias).or_insert_with(|| Some(id.clone()));
                if target.as_deref().is_some_and(|target| target != id) {
                    *target = None;
                }
            }
            if node.get("type").and_then(Kv::str) != Some("INSTANCE") {
                pending.extend(self.children(&id).into_iter().map(|child| (child, depth + 1)));
            }
        }
        // A scope cut short could name the wrong layer for a key: use none of the keys.
        if !complete {
            aliases.clear();
        }
        let scope = Rc::new(OverrideScope { current, aliases });
        let cost = scope.current.len() + scope.aliases.len();
        if cost <= self.max_nodes.saturating_sub(self.override_scope_entries) {
            self.override_scope_entries += cost;
            self.override_scopes.insert(main.to_string(), Rc::clone(&scope));
        }
        scope
    }

    /// The DOCUMENT with its pages.
    fn document(&mut self) -> Value {
        let doc_id = self.nodes.iter().find(|(_, n)| n.get("type").and_then(Kv::str) == Some("DOCUMENT")).map(|(id, _)| id.clone());
        let pages: Vec<String> = match &doc_id {
            Some(d) => self.children(d),
            None => self.nodes.iter().filter(|(_, n)| n.get("type").and_then(Kv::str) == Some("CANVAS")).map(|(id, _)| id.clone()).collect(),
        };
        let mut out = Vec::new();
        let mut internal = Vec::new();
        let mut visible_pages = Vec::new();
        for p in pages {
            let Some(&n) = self.nodes.get(&p) else { continue };
            if n.get("type").and_then(Kv::str) != Some("CANVAS") {
                self.warn("document children other than pages were skipped");
                continue;
            }
            // The hidden page that holds styles, variables and library copies.
            if n.get("internalOnly").and_then(Kv::bool) == Some(true) {
                internal.push((p, n));
                continue;
            }
            visible_pages.push(p.clone());
            out.push(self.page(&p, n));
        }
        let required_mains = self.referenced_component_mains(&visible_pages);
        for (id, canvas) in internal {
            if let Some(context) = self.library_context(&id, canvas, &required_mains) {
                self.library_contexts.push(context);
            }
        }
        if out.is_empty() {
            self.warn("the file has no pages");
        }
        let mut doc = json!({"id": doc_id.unwrap_or_else(|| "0:0".into()), "type": "DOCUMENT"});
        doc["children"] = Value::Array(out);
        doc
    }

    /// The ids of the components the visible pages use, directly or through the components they
    /// use in turn (nested instances, swap targets, overrides naming another main).
    fn referenced_component_mains(&mut self, visible_pages: &[String]) -> BTreeSet<String> {
        let mut mains = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut pending: Vec<(String, usize)> = visible_pages.iter().flat_map(|page| self.children(page)).map(|id| (id, 1)).collect();
        while let Some((id, depth)) = pending.pop() {
            if depth > MAX_DEPTH {
                self.warn(format!("component references below {MAX_DEPTH} layer levels were skipped"));
                continue;
            }
            if !visited.insert(id.clone()) {
                continue;
            }
            if visited.len() > self.max_nodes {
                let max = self.max_nodes;
                self.warn(format!("the component reference scan reached the {max}-layer limit"));
                break;
            }
            let Some(node) = self.nodes.get(&id) else { continue };
            for target in self.component_main_targets(node) {
                if mains.insert(target.clone()) {
                    pending.push((target, 1));
                }
            }
            pending.extend(self.children(&id).into_iter().map(|child| (child, depth + 1)));
        }
        mains
    }

    /// The mains a layer names: an instance's own, the ones its overrides swap in, the instance-swap
    /// defaults and values of component properties.
    fn component_main_targets(&self, node: &Kv<'a>) -> Vec<String> {
        let mut targets: Vec<String> = node
            .get("componentPropDefs")
            .map(Kv::list)
            .unwrap_or_default()
            .iter()
            .filter_map(|definition| {
                self.is_instance_swap(definition.get("id")?)?;
                definition.get("initialValue")?.get("guidValue").and_then(guid)
            })
            .collect();
        if node.get("type").and_then(Kv::str) != Some("INSTANCE") {
            return targets;
        }
        // The instance and the override rows below it (rows carry the same fields, nested).
        let mut pending: Vec<(&Kv<'a>, usize)> = vec![(node, 0)];
        let mut rows = 0usize;
        while let Some((row, depth)) = pending.pop() {
            rows += 1;
            if depth > MAX_DEPTH || rows > self.max_nodes {
                break;
            }
            let data = row.get("symbolData");
            targets.extend(row.get("overriddenSymbolID").and_then(guid));
            targets.extend(data.and_then(|d| d.get("symbolID")).and_then(guid));
            for assignment in row.get("componentPropAssignments").map(Kv::list).unwrap_or_default() {
                if assignment.get("defID").is_some_and(|def| self.is_instance_swap(def).is_some()) {
                    targets.extend(assignment.get("value").and_then(|v| v.get("guidValue")).and_then(guid));
                }
            }
            let overrides = data.and_then(|d| d.get("symbolOverrides")).map(Kv::list).unwrap_or_default();
            let derived = row.get("derivedSymbolData").map(Kv::list).unwrap_or_default();
            pending.extend(overrides.iter().chain(derived).map(|row| (row, depth + 1)));
        }
        targets
    }

    fn is_instance_swap(&self, def: &Kv) -> Option<()> {
        matches!(self.prop_defs.get(&guid(def)?), Some((_, "INSTANCE_SWAP"))).then_some(())
    }

    /// The branches of the internal canvas `id` that hold the `required_mains` (with the frames
    /// they sit in), as a frame of its own: the cached definitions of components that come from a
    /// library. `None` when it holds none.
    fn library_context(&mut self, id: &str, canvas: &'a Kv<'a>, required_mains: &BTreeSet<String>) -> Option<Value> {
        let mut included = BTreeSet::new();
        let mut parents: BTreeMap<String, String> = BTreeMap::new();
        // The canvas itself is never a layer of the walk, whatever the parent links say.
        let mut visited = BTreeSet::from([id.to_string()]);
        // Each entry: layer, depth, inside a required definition.
        let mut pending: Vec<(String, usize, bool)> = self.children(id).into_iter().map(|child| (child, 1, false)).collect();
        while let Some((node_id, depth, in_definition)) = pending.pop() {
            if depth > MAX_DEPTH {
                self.warn(format!("internal library branches deeper than {MAX_DEPTH} levels were skipped"));
                continue;
            }
            if !visited.insert(node_id.clone()) {
                continue;
            }
            self.library_scan_count += 1;
            if self.library_scan_count > self.max_nodes {
                let max = self.max_nodes;
                self.warn(format!("the internal library scan reached the {max}-layer limit; the remaining branches were skipped"));
                break;
            }
            let Some(node) = self.nodes.get(&node_id) else { continue };
            let kind = node.get("type").and_then(Kv::str);
            let definition = (kind == Some("SYMBOL") || (kind == Some("FRAME") && node.get("isStateGroup").and_then(Kv::bool) == Some(true)))
                && required_mains.contains(&node_id);
            let in_definition = in_definition || definition;
            if in_definition {
                included.insert(node_id.clone());
            }
            if definition {
                // The frames around it keep the path its layers are addressed by.
                let mut ancestor = parents.get(&node_id);
                for _ in 0..=MAX_DEPTH {
                    let Some(parent) = ancestor else { break };
                    if !included.insert(parent.clone()) {
                        break;
                    }
                    ancestor = parents.get(parent);
                }
            }
            for child in self.children(&node_id).into_iter().rev() {
                // A layer reached again keeps the parent it was first reached by, so the parents
                // lead back to the canvas.
                if visited.contains(&child) {
                    continue;
                }
                parents.entry(child.clone()).or_insert_with(|| node_id.clone());
                pending.push((child, depth + 1, in_definition));
            }
        }
        if included.is_empty() {
            return None;
        }
        let mut context = Obj::new();
        context.insert("id".into(), json!(id));
        context.insert("type".into(), json!("FRAME"));
        copy_str(&mut context, &View::of(canvas), "name", "name");
        context.insert("size".into(), json!({"x": 0.0, "y": 0.0}));
        context.insert("relativeTransform".into(), rows(Affine::IDENTITY));
        context.insert(crate::rest::FIG_SOURCE_FIELD.into(), json!({"origin": id, "expanded": false, "wireFrame": true, "wireInstance": false}));
        self.variable_bindings(&mut context, &View::of(canvas));
        let mut children = Vec::new();
        for child in self.children(id) {
            children.extend(self.layer_filtered(&child, None, Affine::IDENTITY, None, None, 0, Some(&included)));
        }
        if !children.is_empty() {
            context.insert("children".into(), Value::Array(children));
        }
        Some(Value::Object(context))
    }

    fn page(&mut self, id: &str, n: &'a Kv<'a>) -> Value {
        let v = View::of(n);
        let mut o = Obj::new();
        o.insert("id".into(), json!(id));
        o.insert("type".into(), json!("CANVAS"));
        copy_str(&mut o, &v, "name", "name");
        if let Some(c) = v.get("backgroundColor") {
            o.insert("backgroundColor".into(), color(c));
        }
        if let Some(c) = v.get("prototypeBackgroundColor") {
            o.insert("prototypeBackgrounds".into(), json!([color(c)]));
        }
        self.variable_bindings(&mut o, &v);
        let mut flows: Vec<(&str, Value)> = Vec::new();
        let mut kids = Vec::new();
        for c in self.children(id) {
            if let Some(sp) = self.nodes.get(&c).and_then(|k| k.get("prototypeStartingPoint")) {
                let name = sp.get("name").and_then(Kv::str).filter(|s| !s.is_empty()).unwrap_or("Flow");
                flows.push((sp.get("position").and_then(Kv::str).unwrap_or(""), json!({"nodeId": c, "name": name})));
            }
            kids.extend(self.layer(&c, None, Affine::IDENTITY, None, None, 0));
        }
        flows.sort_by(|a, b| a.0.cmp(b.0));
        if !flows.is_empty() {
            o.insert("flowStartingPoints".into(), Value::Array(flows.into_iter().map(|f| f.1).collect()));
        }
        o.insert("children".into(), Value::Array(kids));
        Value::Object(o)
    }

    /// Layer `id` (inside instance `inst` when expanding one) and its subtree. `space`: from the
    /// parent's coordinates to the enclosing frame's (groups and booleans place their children in
    /// the frame's space, like the REST JSON). `parent`: the parent's view (grid tracks).
    fn layer(
        &mut self,
        id: &str,
        inst: Option<&Inst<'a>>,
        space: Affine,
        parent: Option<&View<'a>>,
        fit: Option<Fit>,
        depth: usize,
    ) -> Option<Value> {
        self.layer_filtered(id, inst, space, parent, fit, depth, None)
    }

    /// [`Self::layer`], keeping only the layers in `included` (all when `None`) below the
    /// top-level one: the cached library definitions of an internal canvas.
    fn layer_filtered(
        &mut self,
        id: &str,
        inst: Option<&Inst<'a>>,
        space: Affine,
        parent: Option<&View<'a>>,
        fit: Option<Fit>,
        depth: usize,
        included: Option<&BTreeSet<String>>,
    ) -> Option<Value> {
        if included.is_some_and(|ids| !ids.contains(id)) {
            return None;
        }
        if depth > MAX_DEPTH {
            self.warn(format!("layers nested deeper than {MAX_DEPTH} levels were skipped"));
            return None;
        }
        if self.count >= self.max_nodes {
            let max = self.max_nodes;
            self.warn(format!("the file has more than {max} layers; the rest were skipped"));
            return None;
        }
        let base = *self.nodes.get(id)?;
        let path = inst.map(|i| [i.path.as_slice(), &[id.to_string()]].concat());
        let entries: &[OverrideRow<'a>] = match (inst, &path) {
            (Some(i), Some(p)) => i.overrides.get(p).map(Vec::as_slice).unwrap_or_default(),
            _ => &[],
        };
        let mut view = View::of(base).with(entries.iter().map(|e| e.row));
        // A layer of a resized instance the file gives no derived layout for follows its
        // constraints from the main's geometry, then takes the size and place overridden on it
        // (auto layout places the others, as in the editor).
        let derived = entries.iter().any(|e| e.entry == Entry::Derived && (e.row.get("size").is_some() || e.row.get("transform").is_some()));
        let stacked = parent.and_then(|p| p.str("stackMode")).is_some_and(|m| matches!(m, "HORIZONTAL" | "VERTICAL" | "GRID"))
            && view.str("stackPositioning") != Some("ABSOLUTE");
        let main = View::of(base);
        let set = |k: &str| entries.iter().rev().filter(|e| e.entry != Entry::Stale).find_map(|e| e.row.get(k));
        if let (Some(f), false, false, Some((w, h))) = (fit, derived, stacked, main.size()) {
            let t = main.transform().unwrap_or_default();
            let (fw, ft) = f.apply(w, h, t, view.str("horizontalConstraint"), view.str("verticalConstraint"));
            view.fitted = Some((set("size").and_then(size).unwrap_or(fw), set("transform").and_then(matrix).unwrap_or(ft)));
        } else if entries.iter().any(|e| e.entry == Entry::Stale && (e.row.get("size").is_some() || e.row.get("transform").is_some()))
            && let Some(s) = set("size").and_then(size).or_else(|| main.size())
        {
            view.fitted = Some((s, set("transform").and_then(matrix).or_else(|| main.transform()).unwrap_or_default()));
        }
        let rest_id = match (inst, &path) {
            (Some(i), Some(p)) => format!("I{};{}", i.root, p.join(";")),
            _ => id.to_string(),
        };
        let own = view.str("type").unwrap_or("");
        let connector_snapshot = own == "CONNECTOR" && self.connector_snapshot_supported(&view);
        // Only a connector with a saved label needs the frame wrapper; an unlabelled one stays a single vector.
        let connector_terminal =
            connector_snapshot && (!connector_has_saved_label(&view) || depth >= MAX_DEPTH || self.count.saturating_add(1) >= self.max_nodes);
        // A supported connector snapshot has its own note below instead of the generic stand-in one.
        let stand = stand_in(own).filter(|_| !connector_snapshot);
        let ty = match own {
            "ROUNDED_RECTANGLE" => "RECTANGLE",
            "SYMBOL" => "COMPONENT",
            "FRAME" if view.flag("isStateGroup") == Some(true) => "COMPONENT_SET",
            "FRAME" if view.flag("resizeToFit") == Some(true) => "GROUP",
            "VARIABLE" | "VARIABLE_SET" => return None,
            "CONNECTOR" if connector_snapshot && connector_terminal => "VECTOR",
            "CONNECTOR" if connector_snapshot => "FRAME",
            t => stand.map_or(t, |s| s.0),
        };
        if let Some((_, what)) = stand.filter(|_| own != "TEXT_PATH") {
            self.warn(format!("{} layers import as {what}", own.to_ascii_lowercase().replace('_', " ")));
        }
        if connector_snapshot {
            self.warn("connector layers import as static frame/vector snapshots with labels when present; live endpoint bindings are not retained");
        }
        self.count += 1;
        let mut o = Obj::new();
        o.insert("id".into(), json!(rest_id));
        // Private provenance for the .fig importer; ordinary REST import ignores it.
        o.insert(
            crate::rest::FIG_SOURCE_FIELD.into(),
            json!({
                "origin": id,
                "expanded": inst.is_some(),
                "wireFrame": own == "FRAME",
                "wireInstance": own == "INSTANCE",
                "slotContent": inst.is_some_and(|i| i.slot_content),
                // A size the instance gives this layer itself (kept when the instance relays out).
                "sizeOverride": entries.iter().any(|e| e.entry == Entry::Override && e.row.get("size").is_some()),
            }),
        );
        o.insert("type".into(), json!(ty));
        copy_str(&mut o, &view, "name", "name");
        self.common(&mut o, &view, ty, space, parent);
        match ty {
            "TEXT" => {
                self.text(&mut o, &view);
                if own == "TEXT_PATH" {
                    self.text_path(&mut o, &view);
                }
            }
            "VECTOR" | "STAR" | "REGULAR_POLYGON" => self.geometry(&mut o, &view),
            "ELLIPSE" => {
                if let Some(a) = view.get("arcData") {
                    let arc: Obj =
                        ["startingAngle", "endingAngle", "innerRadius"].iter().filter_map(|k| Some((k.to_string(), json!(num(a, k)?)))).collect();
                    o.insert("arcData".into(), Value::Object(arc));
                }
            }
            "BOOLEAN_OPERATION" => {
                if let Some(op) = view.str("booleanOperation") {
                    o.insert("booleanOperation".into(), json!(if op == "XOR" { "EXCLUDE" } else { op }));
                }
            }
            _ => {}
        }
        if own == "CONNECTOR" && connector_snapshot {
            if connector_terminal {
                self.connector_snapshot_flatten(&mut o, &view, &rest_id, space);
            } else {
                o.insert("fills".into(), json!([]));
                o.insert("strokes".into(), json!([]));
                for key in ["styles", "strokeWeight", "strokeAlign", "strokeJoin", "strokeDashes", "strokeCap"] {
                    o.remove(key);
                }
                o.insert("clipsContent".into(), json!(false));
            }
        } else if own == "CONNECTOR" {
            self.connector(&mut o, &view, space);
        }
        if ty == "COMPONENT" || ty == "COMPONENT_SET" {
            self.component(&mut o, &view, &rest_id);
        }
        self.prop_refs(&mut o, &view, inst);
        let slot_content = if own == "FRAME" {
            prop_ref(&view, "SLOT_CONTENT_ID").and_then(|def| inst.and_then(|i| i.slot_contents.get(&def).cloned()))
        } else {
            None
        };
        if slot_content.is_some()
            && let Some(source) = o.get_mut(crate::rest::FIG_SOURCE_FIELD).and_then(Value::as_object_mut)
        {
            source.insert("slotOverride".into(), json!(true));
        }
        let child_space = if matches!(ty, "GROUP" | "BOOLEAN_OPERATION") { space * view.transform().unwrap_or_default() } else { Affine::IDENTITY };
        let mut kids = if connector_snapshot && !connector_terminal { self.connector_snapshot_children(&view, &rest_id) } else { Vec::new() };
        if connector_terminal {
            // The saved vector is flattened into the connector node when no child slot remains.
        } else if ty == "INSTANCE" {
            if let Some((sub, sym)) = self.instance(&mut o, &view, id, inst, &rest_id) {
                let main = self.nodes.get(&sym).map(|m| View::of(m));
                let fit = Fit::frame(main.as_ref().and_then(View::size), view.size());
                // An instance that records no auto layout of its own has its main's.
                let view = match main {
                    Some(m) if view.get("stackMode").is_none() && m.get("stackMode").is_some() => {
                        let layered = View { fitted: view.fitted, ..m.with(view.layers.iter().copied()) };
                        self.auto_layout(&mut o, &layered);
                        layered
                    }
                    _ => view.clone(),
                };
                for c in self.children(&sym) {
                    // The main's branch can live outside the cached context being mapped.
                    kids.extend(self.layer_filtered(&c, Some(&sub), child_space, Some(&view), fit, depth + 1, None));
                }
            }
        } else if let Some(content) = slot_content {
            let mut content_inst = inst.cloned();
            if let Some(content_inst) = &mut content_inst {
                content_inst.slot_content = true;
            }
            for c in self.children(&content) {
                kids.extend(self.layer_filtered(&c, content_inst.as_ref(), child_space, Some(&view), None, depth + 1, included));
            }
        } else {
            // Inside an instance a layer's children are its main's: they follow it from the main's
            // size to its own.
            let fit = match (inst, ty) {
                (None, _) => None,
                (Some(_), "GROUP" | "BOOLEAN_OPERATION") => Fit::scale(View::of(base).size(), view.size()),
                (Some(_), _) => Fit::frame(View::of(base).size(), view.size()),
            };
            for c in self.children(id) {
                kids.extend(self.layer_filtered(&c, inst, child_space, Some(&view), fit, depth + 1, included));
            }
        }
        if stand.is_some_and(|s| s.0 == "FRAME") {
            kids.splice(0..0, self.shape_with_text(&mut o, &view, own, &rest_id));
            kids.extend(self.inner_text(&o, &view, &rest_id, own == "STICKY"));
        }
        if !kids.is_empty() {
            o.insert("children".into(), Value::Array(kids));
        }
        Some(Value::Object(o))
    }

    /// A non-rectangular shape-with-text's shape, as a vector under its frame (which gives up its
    /// paints to it).
    fn shape_with_text(&mut self, o: &mut Obj, v: &View<'a>, own: &str, rest_id: &str) -> Option<Value> {
        let shape = v.str("shapeWithTextType").unwrap_or("SQUARE");
        let outlined = !v.list("fillGeometry").is_empty() || !v.list("strokeGeometry").is_empty();
        if own != "SHAPE_WITH_TEXT" || matches!(shape, "SQUARE" | "ROUNDED_RECTANGLE") || !outlined || self.count >= self.max_nodes {
            return None;
        }
        let mut s = Obj::new();
        s.insert("id".into(), json!(format!("{rest_id};shape")));
        s.insert("type".into(), json!("VECTOR"));
        s.insert("name".into(), json!(shape.to_ascii_lowercase().replace('_', " ")));
        for k in ["size", "strokeWeight", "strokeAlign", "strokeJoin", "strokeDashes"] {
            if let Some(x) = o.get(k) {
                s.insert(k.into(), x.clone());
            }
        }
        for k in ["fills", "strokes"] {
            s.insert(k.into(), o.insert(k.into(), json!([])).unwrap_or_else(|| json!([])));
        }
        s.insert("relativeTransform".into(), rows(Affine::IDENTITY));
        self.geometry(&mut s, v);
        if !s.contains_key("fillGeometry") && !s.contains_key("strokeGeometry") {
            // Damaged outlines: the frame keeps its paints and box.
            for k in ["fills", "strokes"] {
                o.insert(k.into(), s.remove(k).unwrap_or_else(|| json!([])));
            }
            return None;
        }
        self.count += 1;
        Some(Value::Object(s))
    }

    /// The text a stand-in frame holds (a sticky note's, a shape's, a table cell's) as a text
    /// layer filling it, inset a little; centred unless it's a note.
    fn inner_text(&mut self, o: &Obj, v: &View<'a>, rest_id: &str, note: bool) -> Option<Value> {
        let chars = own_text(v)?;
        if self.count >= self.max_nodes {
            return None;
        }
        self.count += 1;
        let size = |k: &str| o.get("size").and_then(|s| s.get(k)).and_then(Value::as_f64).unwrap_or(0.0);
        let (w, h) = (size("x"), size("y"));
        let pad = (w.min(h) / 4.0).clamp(0.0, 16.0);
        let mut t = Obj::new();
        t.insert("id".into(), json!(format!("{rest_id};text")));
        t.insert("type".into(), json!("TEXT"));
        t.insert("name".into(), json!(chars.chars().take(40).collect::<String>()));
        t.insert("size".into(), json!({"x": (w - 2.0 * pad).max(1.0), "y": (h - 2.0 * pad).max(1.0)}));
        t.insert("relativeTransform".into(), rows(Affine::translate((pad, pad))));
        self.text(&mut t, v);
        if let Some(Value::Object(st)) = t.get_mut("style") {
            let (hor, ver) = if note { ("LEFT", "TOP") } else { ("CENTER", "CENTER") };
            st.entry("textAlignHorizontal").or_insert_with(|| json!(hor));
            st.entry("textAlignVertical").or_insert_with(|| json!(ver));
            st.insert("textAutoResize".into(), json!("NONE"));
        }
        Some(Value::Object(t))
    }

    /// A connector's end caps, and when it has no saved outline or network a straight line
    /// between its ends: free points, or a point (the centre unless given) of a sibling layer.
    fn connector(&mut self, o: &mut Obj, v: &View<'a>, space: Affine) {
        for (from, to) in [("connectorStartCap", "strokeCapStart"), ("connectorEndCap", "strokeCapEnd")] {
            if let Some(c) = v.str(from) {
                o.insert(to.into(), json!(cap_name(c)));
            }
        }
        if connector_has_saved_label(v) {
            self.warn("connector labels were skipped");
        }
        if ["vectorNetwork", "vectorNetworkRef", "fillGeometry", "strokeGeometry"].iter().any(|k| o.contains_key(*k)) {
            return;
        }
        let parent = v.get("parentIndex").and_then(|p| p.get("guid")).and_then(guid);
        let end = |k: &str| -> Option<Point> {
            let e = v.get(k)?;
            let p = match e.get("endpointNodeID").and_then(guid).filter(|g| g != NO_STYLE) {
                Some(id) => {
                    let n = *self.nodes.get(&id)?;
                    // Only siblings share the connector's coordinates.
                    if n.get("parentIndex").and_then(|p| p.get("guid")).and_then(guid) != parent {
                        return None;
                    }
                    let (w, h) = n.get("size").map_or((0.0, 0.0), |s| (num(s, "x").unwrap_or(0.0), num(s, "y").unwrap_or(0.0)));
                    let (rx, ry) = e.get("relativePosition").and_then(|r| Some((num(r, "x")?, num(r, "y")?))).unwrap_or((0.5, 0.5));
                    n.get("transform").and_then(matrix).unwrap_or_default() * Point::new(w * rx, h * ry)
                }
                None => e.get("position").and_then(|p| Some(Point::new(num(p, "x")?, num(p, "y")?)))?,
            };
            (p.x.is_finite() && p.y.is_finite()).then_some(p)
        };
        let (Some(a), Some(b)) = (end("connectorStart"), end("connectorEnd")) else {
            self.warn("connectors whose ends can't be placed are empty");
            return;
        };
        if v.str("connectorLineStyle").is_some_and(|s| s != "STRAIGHT") {
            self.warn("elbowed and curved connectors without a saved path import as straight lines");
        }
        let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
        o.insert("relativeTransform".into(), rows(space * Affine::translate((x0, y0))));
        o.insert("size".into(), json!({"x": (a.x - b.x).abs(), "y": (a.y - b.y).abs()}));
        o.insert(
            "vectorNetwork".into(),
            json!({"vertices": [{"x": a.x - x0, "y": a.y - y0}, {"x": b.x - x0, "y": b.y - y0}], "segments": [{"a": 0, "b": 1}]}),
        );
    }

    fn connector_snapshot_supported(&self, v: &View<'a>) -> bool {
        if v.str("connectorType") != Some("MANUAL")
            || v.get("connectorVersion").and_then(Kv::f64) != Some(1.0)
            || v.str("connectorLineStyle") != Some("STRAIGHT")
            || v.get("derivedImmutableFrameData").and_then(|data| data.get("version")).and_then(Kv::f64) != Some(1.0)
        {
            return false;
        }
        let Some(line) = connector_record(v, "derivedImmutableFrameData", 0) else { return false };
        if line.get("transform").and_then(matrix).is_none() || valid_size(line.get("size")).is_none() {
            return false;
        }
        let geometry = ["fillGeometry", "strokeGeometry"].iter().any(|key| {
            line.get(key).map(Kv::list).unwrap_or_default().iter().any(|path| {
                num(path, "commandsBlob")
                    .and_then(|index| self.blobs.get(index as usize))
                    .and_then(|blob| svg_path(blob))
                    .is_some_and(|data| !data.is_empty())
            })
        });
        let network = line
            .get("vectorData")
            .and_then(|data| num(data, "vectorNetworkBlob"))
            .and_then(|index| self.blobs.get(index as usize))
            .and_then(|blob| network::decode(blob))
            .is_some_and(|network| !network.segments.is_empty());
        (geometry || network) && connector_record(v, "nodeGenerationData", 0).is_some()
    }

    fn connector_snapshot_flatten(&mut self, out: &mut Obj, v: &View<'a>, rest_id: &str, space: Affine) {
        let parent = space * v.get("transform").and_then(matrix).unwrap_or_default();
        let Some(vector) = self.connector_snapshot_vector(v, rest_id, parent, false) else {
            self.warn("connector snapshot vector data is missing; connector was left empty");
            return;
        };
        for key in [
            "size",
            "relativeTransform",
            "fills",
            "strokes",
            "strokeWeight",
            "strokeAlign",
            "strokeJoin",
            "strokeDashes",
            "strokeCap",
            "strokeCapStart",
            "strokeCapEnd",
            "styles",
            "fillGeometry",
            "strokeGeometry",
            "vectorNetwork",
        ] {
            if let Some(value) = vector.get(key) {
                out.insert(key.to_string(), value.clone());
            }
        }
        if vector.get("visible").and_then(Value::as_bool) == Some(false) {
            out.insert("visible".into(), json!(false));
        }
        if let Some(line_opacity) = vector.get("opacity").and_then(Value::as_f64) {
            let root_opacity = out.get("opacity").and_then(Value::as_f64).unwrap_or(1.0);
            out.insert("opacity".into(), json!(root_opacity * line_opacity));
        }
        if connector_has_saved_label(v) {
            let reason = if self.count >= self.max_nodes { "node budget" } else { "depth limit" };
            self.warn(format!("connector label was skipped: {reason} prevents nested snapshot children"));
        }
    }

    /// Build the saved vector child or flattened terminal connector from the same child-only view.
    fn connector_snapshot_vector(&mut self, v: &View<'a>, rest_id: &str, space: Affine, nested: bool) -> Option<Obj> {
        let (Some(line), Some(line_style)) = (connector_record(v, "derivedImmutableFrameData", 0), connector_record(v, "nodeGenerationData", 0))
        else {
            return None;
        };
        let mut vector = Obj::new();
        vector.insert("id".into(), json!(if nested { format!("{rest_id};vector") } else { rest_id.to_string() }));
        vector.insert("type".into(), json!("VECTOR"));
        if let Some(name) = v.str("name") {
            vector.insert("name".into(), json!(if nested { format!("{name} vector") } else { name.to_string() }));
        }
        let line_view = View::of(line).with([line_style]);
        self.common(&mut vector, &line_view, "VECTOR", space, None);
        self.geometry(&mut vector, &line_view);
        for (from, to) in [("connectorStartCap", "strokeCapStart"), ("connectorEndCap", "strokeCapEnd")] {
            if let Some(cap) = v.str(from) {
                vector.entry(to).or_insert_with(|| json!(cap_name(cap)));
            }
        }
        Some(vector)
    }

    /// Snapshot the straight, manual connector contract carried by its generated children.
    fn connector_snapshot_children(&mut self, v: &View<'a>, rest_id: &str) -> Vec<Value> {
        if self.count >= self.max_nodes {
            self.warn(format!("the file has more than {} layers; connector snapshot children were skipped", self.max_nodes));
            return Vec::new();
        }
        let Some(vector) = self.connector_snapshot_vector(v, rest_id, Affine::IDENTITY, true) else {
            self.warn("connector snapshot vector data is missing; connector was left empty");
            return Vec::new();
        };
        let mut children = Vec::new();
        children.push(Value::Object(vector));
        self.count += 1;

        let Some(text_frame) = connector_record(v, "derivedImmutableFrameData", 1) else {
            if connector_has_saved_label(v) {
                self.warn("connector label was skipped: derived text bounds are missing");
            }
            return children;
        };
        let Some(background) = connector_record(v, "derivedImmutableFrameData", 2) else {
            if connector_has_saved_label(v) || valid_size(text_frame.get("size")).is_some_and(|(w, h)| w > 0.0 && h > 0.0) {
                self.warn("connector label was skipped: derived background bounds are missing");
            }
            return children;
        };
        let Some(text_style) = connector_record(v, "nodeGenerationData", 1) else {
            if valid_size(text_frame.get("size")).is_some_and(|(w, h)| w > 0.0 && h > 0.0) {
                self.warn("connector label was skipped: text style data is missing");
            }
            return children;
        };
        let text_view = View::of(text_frame).with([text_style]);
        let Some(characters) = text_view.get("textData").and_then(|d| d.get("characters")).and_then(Kv::str).filter(|s| !s.trim().is_empty()) else {
            return children;
        };
        let Some(text_transform) = text_frame.get("transform").and_then(matrix) else {
            self.warn("connector label was skipped: text transform is invalid");
            return children;
        };
        let Some(background_transform) = background.get("transform").and_then(matrix) else {
            self.warn("connector label was skipped: background transform is invalid");
            return children;
        };
        let Some(text_size) = valid_size(text_frame.get("size")) else {
            self.warn("connector label was skipped: text bounds are invalid");
            return children;
        };
        let Some(background_size) = valid_size(background.get("size")) else {
            self.warn("connector label was skipped: background bounds are invalid");
            return children;
        };
        let composed_text_transform = background_transform * text_transform;
        if !composed_text_transform.as_coeffs().iter().all(|coefficient| coefficient.is_finite()) {
            self.warn("connector label was skipped: composed text transform is invalid");
            return children;
        }

        let Some(background_style) = connector_record(v, "nodeGenerationData", 2) else {
            self.warn("connector label background was skipped: style data is missing");
            self.add_connector_text(&mut children, rest_id, characters, text_size, composed_text_transform, &text_view);
            return children;
        };
        let background_view = View::of(background).with([background_style]);
        if !background_view.list("fillPaints").is_empty() {
            if self.count >= self.max_nodes {
                self.warn(format!("the file has more than {} layers; connector label background was skipped", self.max_nodes));
            } else {
                let mut rect = Obj::new();
                rect.insert("id".into(), json!(format!("{rest_id};label-background")));
                rect.insert("type".into(), json!("RECTANGLE"));
                rect.insert("name".into(), json!("connector label background"));
                self.common(&mut rect, &background_view, "RECTANGLE", Affine::IDENTITY, None);
                rect.insert("size".into(), json!({"x": background_size.0, "y": background_size.1}));
                rect.insert("relativeTransform".into(), rows(background_transform));
                rect.insert("clipsContent".into(), json!(false));
                children.push(Value::Object(rect));
                self.count += 1;
            }
        }
        self.add_connector_text(&mut children, rest_id, characters, text_size, composed_text_transform, &text_view);
        children
    }

    fn add_connector_text(
        &mut self,
        children: &mut Vec<Value>,
        rest_id: &str,
        characters: &str,
        size: (f64, f64),
        transform: Affine,
        view: &View<'a>,
    ) {
        if self.count >= self.max_nodes {
            self.warn(format!("the file has more than {} layers; connector label was skipped", self.max_nodes));
            return;
        }
        let mut text = Obj::new();
        text.insert("id".into(), json!(format!("{rest_id};label-text")));
        text.insert("type".into(), json!("TEXT"));
        text.insert("name".into(), json!(characters));
        text.insert("size".into(), json!({"x": size.0, "y": size.1}));
        self.common(&mut text, view, "TEXT", Affine::IDENTITY, None);
        text.insert("relativeTransform".into(), rows(transform));
        self.text(&mut text, view);
        children.push(Value::Object(text));
        self.count += 1;
    }

    /// Fields every layer type shares.
    fn common(&mut self, o: &mut Obj, v: &View<'a>, ty: &str, space: Affine, parent: Option<&View<'a>>) {
        for (from, to) in [("visible", "visible"), ("locked", "locked"), ("mask", "isMask")] {
            if let Some(b) = v.flag(from) {
                o.insert(to.into(), json!(b));
            }
        }
        if let Some(m) = v.str("maskType") {
            o.insert("maskType".into(), json!(if m == "OUTLINE" { "VECTOR" } else { m }));
        }
        copy_num(o, v, "opacity", "opacity");
        if let Some(b) = v.str("blendMode").filter(|b| *b != "UNSET") {
            o.insert("blendMode".into(), json!(b));
        }
        if let Some((x, y)) = v.size() {
            o.insert("size".into(), json!({"x": x, "y": y}));
        }
        let t = v.transform().unwrap_or_default();
        o.insert("relativeTransform".into(), rows(space * t));
        // Paints, from the shared style when one is linked (the layer's own copy can be stale),
        // unless the layer's own paints are bound to variables: those bindings must not be lost.
        let mut styles = Obj::new();
        let own = |k: &str| Some(v.list(k)).filter(|l| l.iter().any(paint_bound));
        let fill_style = self.style(v, "styleIdForFill", "inheritFillStyleID", "fill", &mut styles);
        let fills = own("fillPaints")
            .unwrap_or_else(|| fill_style.map_or_else(|| v.list("fillPaints"), |s| s.get("fillPaints").map(Kv::list).unwrap_or_default()));
        o.insert("fills".into(), Value::Array(fills.iter().filter_map(|p| paint(p, &mut |w| self.warn(w))).collect()));
        let stroke_style = self.style(v, "styleIdForStrokeFill", "inheritFillStyleIDForStroke", "stroke", &mut styles);
        let strokes = own("strokePaints")
            .unwrap_or_else(|| stroke_style.map_or_else(|| v.list("strokePaints"), |s| s.get("fillPaints").map(Kv::list).unwrap_or_default()));
        o.insert("strokes".into(), Value::Array(strokes.iter().filter_map(|p| paint(p, &mut |w| self.warn(w))).collect()));
        let effect_style = self.style(v, "styleIdForEffect", "inheritEffectStyleID", "effect", &mut styles);
        let effects = effect_style.map_or_else(|| v.list("effects"), |s| s.get("effects").map(Kv::list).unwrap_or_default());
        if !effects.is_empty() {
            o.insert("effects".into(), Value::Array(effects.iter().filter_map(effect).collect()));
        }
        let grid_style = self.style(v, "styleIdForGrid", "inheritGridStyleID", "grid", &mut styles);
        let grids = grid_style.map_or_else(|| v.list("layoutGrids"), |s| s.get("layoutGrids").map(Kv::list).unwrap_or_default());
        if !grids.is_empty() {
            o.insert("layoutGrids".into(), Value::Array(grids.iter().map(layout_grid).collect()));
        }
        if ty == "TEXT" {
            self.style(v, "styleIdForText", "inheritTextStyleID", "text", &mut styles);
        }
        if !styles.is_empty() {
            o.insert("styles".into(), Value::Object(styles));
        }
        stroke(o, v);
        copy_num(o, v, "cornerRadius", "cornerRadius");
        copy_num(o, v, "cornerSmoothing", "cornerSmoothing");
        if v.flag("rectangleCornerRadiiIndependent") == Some(true) {
            let r = |k: &str| v.num(k).unwrap_or(0.0);
            o.insert(
                "rectangleCornerRadii".into(),
                json!([
                    r("rectangleTopLeftCornerRadius"),
                    r("rectangleTopRightCornerRadius"),
                    r("rectangleBottomRightCornerRadius"),
                    r("rectangleBottomLeftCornerRadius")
                ]),
            );
        }
        let constraint = |k: &str, min: &'static str, max: &'static str, both: &'static str| match v.str(k) {
            Some("CENTER") => "CENTER",
            Some("MAX" | "FIXED_MAX") => max,
            Some("STRETCH") => both,
            Some("SCALE") => "SCALE",
            _ => min,
        };
        o.insert(
            "constraints".into(),
            json!({
                "horizontal": constraint("horizontalConstraint", "LEFT", "RIGHT", "LEFT_RIGHT"),
                "vertical": constraint("verticalConstraint", "TOP", "BOTTOM", "TOP_BOTTOM"),
            }),
        );
        if matches!(ty, "FRAME" | "COMPONENT" | "COMPONENT_SET" | "INSTANCE" | "SECTION") {
            o.insert("clipsContent".into(), json!(!v.flag("frameMaskDisabled").unwrap_or(false)));
            self.auto_layout(o, v);
        }
        child_layout(o, v, parent);
        let exports: Vec<Value> = v.list("exportSettings").iter().map(export_setting).collect();
        if !exports.is_empty() {
            o.insert("exportSettings".into(), Value::Array(exports));
        }
        if let Some(b) = v.flag("sectionContentsHidden") {
            o.insert("sectionContentsHidden".into(), json!(b));
        }
        self.prototype(o, v);
        self.variable_bindings(o, v);
    }

    /// The style node a layer links in `key` (or the older `legacy` field), recorded for the REST
    /// `styles` map under `slot`.
    fn style(&mut self, v: &View<'a>, key: &str, legacy: &str, slot: &str, out: &mut Obj) -> Option<&'a Kv<'a>> {
        let id = style_link(v, key, legacy)?;
        let node = *self.nodes.get(&id)?;
        node.get("styleType").and_then(Kv::str)?;
        out.insert(slot.into(), json!(id));
        if !self.styles.contains_key(&id) {
            let name = node.get("name").and_then(Kv::str).unwrap_or("Style");
            let desc = node.get("styleDescription").or_else(|| node.get("description")).and_then(Kv::str).unwrap_or("");
            self.styles.insert(id, json!({"name": name, "description": desc}));
        }
        Some(node)
    }

    fn auto_layout(&mut self, o: &mut Obj, v: &View<'a>) {
        let Some(mode) = v.str("stackMode").filter(|m| matches!(*m, "HORIZONTAL" | "VERTICAL" | "GRID")) else { return };
        o.insert("layoutMode".into(), json!(mode));
        copy_num(o, v, "stackSpacing", "itemSpacing");
        copy_num(o, v, "stackCounterSpacing", "counterAxisSpacing");
        if v.str("stackWrap") == Some("WRAP") {
            o.insert("layoutWrap".into(), json!("WRAP"));
        }
        if v.str("stackCounterAlignContent") == Some("SPACE_BETWEEN") {
            o.insert("counterAxisAlignContent".into(), json!("SPACE_BETWEEN"));
        }
        let (h, vert) = (v.num("stackHorizontalPadding").unwrap_or(0.0), v.num("stackVerticalPadding").unwrap_or(0.0));
        o.insert("paddingLeft".into(), json!(h));
        o.insert("paddingTop".into(), json!(vert));
        o.insert("paddingRight".into(), json!(v.num("stackPaddingRight").unwrap_or(h)));
        o.insert("paddingBottom".into(), json!(v.num("stackPaddingBottom").unwrap_or(vert)));
        let main = match v.str("stackPrimaryAlignItems").or_else(|| v.str("stackJustify")) {
            Some("CENTER") => "CENTER",
            Some("MAX") => "MAX",
            Some("SPACE_AROUND") => "SPACE_AROUND",
            // `SPACE_EVENLY` is the format's older name for equal gaps without edge space.
            Some("SPACE_EVENLY_CSS") => "SPACE_EVENLY",
            Some("SPACE_BETWEEN" | "SPACE_EVENLY") => "SPACE_BETWEEN",
            _ => "MIN",
        };
        o.insert("primaryAxisAlignItems".into(), json!(main));
        let cross = match v.str("stackCounterAlignItems").or_else(|| v.str("stackAlign")) {
            Some("CENTER") => "CENTER",
            Some("MAX") => "MAX",
            Some("BASELINE") => "BASELINE",
            _ => "MIN",
        };
        o.insert("counterAxisAlignItems".into(), json!(cross));
        let sizing = |k: &str| if v.str(k).is_some_and(|s| s.starts_with("RESIZE_TO_FIT")) { "AUTO" } else { "FIXED" };
        // A flow that records no sizing for its primary axis sizes it by its content: auto layout
        // did before that axis could be fixed.
        let primary = if v.get("stackPrimarySizing").is_none() && mode != "GRID" { "AUTO" } else { sizing("stackPrimarySizing") };
        o.insert("primaryAxisSizingMode".into(), json!(primary));
        o.insert("counterAxisSizingMode".into(), json!(sizing("stackCounterSizing")));
        if let Some(b) = v.flag("bordersTakeSpace") {
            o.insert("strokesIncludedInLayout".into(), json!(b));
        }
        if let Some(b) = v.flag("stackReverseZIndex") {
            o.insert("itemReverseZIndex".into(), json!(b));
        }
        if mode == "GRID" {
            copy_num(o, v, "gridColumnGap", "gridColumnGap");
            copy_num(o, v, "gridRowGap", "gridRowGap");
            o.insert("gridColumnCount".into(), json!(tracks(v, "gridColumns").len()));
            o.insert("gridRowCount".into(), json!(tracks(v, "gridRows").len()));
            let track_data = [("columnTracks", "gridColumns", "gridColumnsSizing"), ("rowTracks", "gridRows", "gridRowsSizing")]
                .into_iter()
                .filter_map(|(out_key, order_key, sizing_key)| {
                    self.map_grid_tracks(v, order_key, sizing_key).map(|mapped| (out_key.to_string(), mapped))
                })
                .collect::<Obj>();
            if !track_data.is_empty() {
                let source = o.entry(crate::rest::FIG_SOURCE_FIELD).or_insert_with(|| json!({}));
                if let Some(source) = source.as_object_mut() {
                    source.insert("gridTracks".into(), Value::Object(track_data));
                }
            }
        }
    }

    /// Track sizing maps are keyed by GUID; their order comes from the separate track list.
    fn map_grid_tracks(&mut self, v: &View<'a>, order_key: &str, sizing_key: &str) -> Option<Value> {
        let track_entries = v.get(order_key).and_then(|m| m.get("entries")).map(Kv::list).unwrap_or_default();
        let sizes = v.get(sizing_key).and_then(|m| m.get("entries")).map(Kv::list).unwrap_or_default();
        if track_entries.len() > MAX_GRID_TRACKS {
            self.warn("grid track list exceeds 1000 entries; the tracks use the default equal sizing");
            return None;
        }
        if sizes.is_empty() {
            return None;
        }
        if sizes.len() > MAX_GRID_TRACKS {
            self.warn("grid track sizing list exceeds 1000 entries; the tracks use the default equal sizing");
            return None;
        }
        let ordered = tracks(v, order_key);
        let ordered_ids: BTreeSet<&str> = ordered.iter().map(String::as_str).collect();
        let mut size_by_id: BTreeMap<String, Option<&Kv<'a>>> = BTreeMap::new();
        for entry in sizes {
            let Some(id) = entry.get("id").and_then(guid) else {
                self.warn("stale or invalid grid track sizing entries were ignored");
                continue;
            };
            if !ordered_ids.contains(id.as_str()) {
                self.warn("stale or invalid grid track sizing entries were ignored");
                continue;
            }
            match size_by_id.entry(id) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(Some(entry));
                }
                std::collections::btree_map::Entry::Occupied(mut slot) => {
                    slot.insert(None);
                    self.warn("duplicate grid track sizing entries use equal tracks");
                }
            }
        }
        let mut mapped = Vec::with_capacity(ordered.len());
        for id in ordered {
            if let Some(Some(entry)) = size_by_id.get(&id) {
                match grid_track_sizing(entry.get("trackSize")) {
                    Some(track) => mapped.push(track),
                    None => {
                        self.warn("grid track sizing is unsupported or malformed; affected tracks use equal sizing");
                        mapped.push(json!({"sizing": "fill", "value": 1.0}));
                    }
                }
            } else {
                mapped.push(json!({"sizing": "fill", "value": 1.0}));
            }
        }
        Some(Value::Array(mapped))
    }

    fn text(&mut self, o: &mut Obj, v: &View<'a>) {
        let style_node = style_link(v, "styleIdForText", "inheritTextStyleID").and_then(|s| self.nodes.get(&s).copied());
        // Instance overrides beat the shared style; the style beats the layer's own copy.
        let sv = v.styled(style_node.filter(|s| s.get("styleType").and_then(Kv::str) == Some("TEXT")));
        let td = v.get("textData");
        let chars = td.and_then(|t| t.get("characters")).and_then(Kv::str).unwrap_or("");
        o.insert("characters".into(), json!(chars));
        let mut style = text_style(&sv, None);
        // The style's typography beats the layer's own copy of it (the copy is stale when the style
        // changed). The file cannot tell a stale copy of these four values from a deliberate local
        // one, and "original", "none" and 0 could never be set locally if the style always won, so
        // a value the layer or an override sets itself is taken as deliberate.
        if let Some(decoration @ ("NONE" | "UNDERLINE" | "STRIKETHROUGH")) = v.str("textDecoration") {
            style.insert("textDecoration".into(), json!(decoration));
        }
        if let Some(skip_ink) = v.flag("textDecorationSkipInk") {
            style.insert("textDecorationSkipInk".into(), json!(skip_ink));
        }
        if let Some(case @ ("ORIGINAL" | "UPPER" | "LOWER" | "TITLE" | "SMALL_CAPS" | "SMALL_CAPS_FORCED")) = v.str("textCase") {
            style.insert("textCase".into(), json!(case));
        }
        for k in ["textAlignHorizontal", "textAlignVertical", "textAutoResize", "textTruncation"] {
            copy_str(&mut style, v, k, k);
        }
        // Paragraph settings belong to the effective text layer, including explicit zero clears.
        for k in ["paragraphSpacing", "paragraphIndent"] {
            copy_num(&mut style, v, k, k);
        }
        insert_text_options(&mut style, v);
        if let Some(n) = v.num("maxLines").filter(|n| *n >= 1.0) {
            style.insert("maxLines".into(), json!(n));
        }
        let size = sv.num("fontSize");
        if let Some(b) = o.get_mut("style").and_then(|s| s.get_mut("boundVariables")).map(Value::take) {
            style.insert("boundVariables".into(), b);
        }
        o.insert("style".into(), Value::Object(style));
        let Some(td) = td else { return };
        char_styles(o, td, size, &mut |w| self.warn(w));
        let raw = td.get("lines").map(Kv::list).unwrap_or_default();
        let lines: Vec<&str> = raw
            .iter()
            .map(|l| match l.get("lineType").and_then(Kv::str) {
                Some("ORDERED_LIST") => "ORDERED",
                Some("UNORDERED_LIST") => "UNORDERED",
                _ => "NONE",
            })
            .collect();
        if lines.iter().any(|l| *l != "NONE") {
            o.insert("lineTypes".into(), json!(lines));
        }
        // Where a numbered list starts counting, less one (not a REST field).
        let starts: Vec<u32> =
            raw.iter().map(|l| num(l, "listStartOffset").filter(|n| n.is_finite()).map_or(0, |n| n.clamp(0.0, f64::from(u32::MAX)) as u32)).collect();
        if starts.iter().any(|n| *n > 0) {
            o.insert("listStartOffsets".into(), json!(starts));
        }
    }

    /// Carry supported text-path data through the private .fig REST ingress only.
    fn text_path(&mut self, o: &mut Obj, v: &View<'a>) {
        let fail = |this: &mut Self, reason: &str| {
            this.warn(format!("text path {reason}; text imports off its path"));
        };
        let Some(start) = v.get("textPathStart") else {
            fail(self, "has no start data");
            return;
        };
        let (Some(t), Some(forward)) = (num(start, "tValue"), start.get("forward").and_then(Kv::bool)) else {
            fail(self, "has invalid start data");
            return;
        };
        let Some(vd) = v.get("vectorData") else {
            fail(self, "has no vector data");
            return;
        };
        let Some(blob) = num(vd, "vectorNetworkBlob")
            .filter(|i| i.is_finite() && *i >= 0.0 && i.fract() == 0.0)
            .and_then(|i| usize::try_from(i as u64).ok())
            .and_then(|i| self.blobs.get(i))
            .copied()
        else {
            fail(self, "has no vector network");
            return;
        };
        let Some(net) = network::decode(blob) else {
            fail(self, "has damaged vector data");
            return;
        };
        let Some(norm) = vd.get("normalizedSize") else {
            fail(self, "has invalid normalized size");
            return;
        };
        let Some(size) = v.get("size") else {
            fail(self, "has invalid dimensions");
            return;
        };
        let (Some(nx), Some(ny), Some(w), Some(h)) = (num(norm, "x"), num(norm, "y"), num(size, "x"), num(size, "y")) else {
            fail(self, "has invalid dimensions");
            return;
        };
        let axis_scale = |size: f64, normalized: f64, values: &[f64]| -> Option<f64> {
            if !size.is_finite() || size < 0.0 || !normalized.is_finite() || normalized < 0.0 {
                return None;
            }
            if normalized > 1e-9 {
                if size <= 0.0 {
                    return None;
                }
                return Some(size / normalized);
            }
            (size == 0.0 && values.iter().all(|v| *v == 0.0)).then_some(1.0)
        };
        let xs: Vec<f64> = net.vertices.iter().map(|p| p.x).chain(net.segments.iter().flat_map(|s| [s.ta.0, s.tb.0])).collect();
        let ys: Vec<f64> = net.vertices.iter().map(|p| p.y).chain(net.segments.iter().flat_map(|s| [s.ta.1, s.tb.1])).collect();
        let sx = axis_scale(w, nx, &xs);
        let sy = axis_scale(h, ny, &ys);
        let (Some(sx), Some(sy)) = (sx, sy) else {
            fail(self, "has invalid path scale");
            return;
        };
        if v.str("textAlignVertical").is_some_and(|x| x != "TOP") {
            fail(self, "uses unsupported vertical placement");
            return;
        }
        if v.get("textData").and_then(|td| td.get("characters")).and_then(Kv::str).is_some_and(|s| s.chars().any(|c| matches!(c, '\n' | '\r'))) {
            fail(self, "contains multiple lines");
            return;
        }
        let path = match super::text_path::from_network(&net, sx, sy, t, forward) {
            Ok(path) => path,
            Err(reason) => {
                fail(self, reason);
                return;
            }
        };
        let has_decoration = |style: &Value| style.get("textDecoration").and_then(Value::as_str).is_some_and(|x| x != "NONE");
        let decorated = o.get("style").is_some_and(has_decoration)
            || o.get("styleOverrideTable").and_then(Value::as_object).into_iter().flat_map(|styles| styles.values()).any(has_decoration);
        if decorated {
            self.warn("text path underline or strikethrough decoration is not rendered");
        }
        o.insert("_figTextPath".into(), json!({"d": path.d, "start": path.start, "flip": path.flip}));
    }

    /// `fillGeometry` / `strokeGeometry` as SVG path data, and star/polygon parameters.
    fn geometry(&mut self, o: &mut Obj, v: &View<'a>) {
        for key in ["fillGeometry", "strokeGeometry"] {
            let mut out = Vec::new();
            for p in v.list(key) {
                let Some(blob) = num(p, "commandsBlob").and_then(|i| self.blobs.get(i as usize)).copied() else { continue };
                let Some(d) = svg_path(blob) else {
                    self.warn("damaged vector geometry was skipped");
                    continue;
                };
                if d.is_empty() {
                    continue;
                }
                let winding = if p.get("windingRule").and_then(Kv::str) == Some("ODD") { "EVENODD" } else { "NONZERO" };
                out.push(json!({"path": d, "windingRule": winding}));
            }
            if !out.is_empty() {
                o.insert(key.into(), Value::Array(out));
            }
        }
        let vector = o.get("type").and_then(Value::as_str) == Some("VECTOR");
        if vector && !self.network(o, v) && !o.contains_key("fillGeometry") && !o.contains_key("strokeGeometry") && v.str("type") != Some("CONNECTOR")
        {
            self.warn("vectors without geometry are empty");
        }
        if !vector {
            copy_num(o, v, "count", "pointCount");
            copy_num(o, v, "starInnerScale", "innerRadius");
        }
    }

    /// The editable network of `vectorData` as `vectorNetwork` (the REST importer's extension),
    /// scaled to the layer's size, with per-point corner radii, handle mirroring and end caps.
    /// False when there's none or it's damaged (the outlines stand in).
    fn network(&mut self, o: &mut Obj, v: &View<'a>) -> bool {
        let Some(vd) = v.get("vectorData") else { return false };
        let Some(blob_id) = num(vd, "vectorNetworkBlob").map(|i| i as usize) else { return false };
        let Some(blob) = self.blobs.get(blob_id).copied() else { return false };
        let Some(net) = network::decode(blob) else {
            self.warn("damaged vector networks import as outlines");
            return false;
        };
        let styles: BTreeMap<u64, &Kv> =
            vd.get("styleOverrideTable").map(Kv::list).unwrap_or_default().iter().filter_map(|s| Some((num(s, "styleID")? as u64, s))).collect();
        let mut has_uninterpreted = false;
        let mut has_missing = false;
        for segment in &net.segments {
            if segment.style == 0 {
                continue;
            }
            if styles.contains_key(&u64::from(segment.style)) {
                has_uninterpreted = true;
            } else {
                has_missing = true;
            }
        }
        if has_uninterpreted {
            self.warn("style settings of some vector segments aren't supported; those segments use the layer's stroke");
        }
        if has_missing {
            self.warn("some vector segments refer to style settings missing from the file; they use the layer's stroke");
        }
        // An open path shows its fill, which regions can't hold: the outline keeps the look.
        let fills_shown =
            o.get("fills").and_then(Value::as_array).is_some_and(|f| f.iter().any(|p| p.get("visible").and_then(Value::as_bool) != Some(false)));
        if net.segments.is_empty() || (net.regions.is_empty() && fills_shown && o.contains_key("fillGeometry")) {
            return false;
        }
        let style = |id: u32| styles.get(&u64::from(id)).copied().filter(|_| id != 0);
        let scale = |k: &str| {
            let size = v.size().map(|s| if k == "x" { s.0 } else { s.1 });
            let norm = vd.get("normalizedSize").and_then(|s| num(s, k));
            size.zip(norm).filter(|(s, n)| s.is_finite() && *n > 1e-9).map_or(1.0, |(s, n)| s / n)
        };
        let (sx, sy) = (scale("x"), scale("y"));
        // An absent layer join is the source default.
        let layer_join = v.str("strokeJoin").unwrap_or("MITER");
        let point_join = |p: &network::Vertex| style(p.style).and_then(|s| s.get("strokeJoin"));
        let known = |j: &str| matches!(j, "MITER" | "BEVEL" | "ROUND");
        let unresolved_style = net.vertices.iter().any(|p| p.style != 0 && style(p.style).is_none());
        let unknown_join = net.vertices.iter().any(|p| point_join(p).is_some_and(|j| !j.str().is_some_and(known)));
        let unsupported_join = net.vertices.iter().any(|p| point_join(p).and_then(Kv::str).is_some_and(|j| known(j) && j != layer_join));
        if net.regions.iter().any(|r| style(r.style).is_some_and(|s| s.get("fillPaints").is_some())) {
            self.warn("filled areas of a vector with their own fill use the layer's fill");
        }
        if unresolved_style {
            self.warn("some vector points refer to point settings missing from the file; they use the layer's joins and plain corners");
        }
        if unknown_join {
            self.warn("vector points with unknown stroke joins use the layer join");
        }
        if unsupported_join && (o.contains_key("strokeDashes") || o.contains_key("strokeWidthProfile")) {
            self.warn("point stroke joins were preserved; dashed or variable-width strokes render with the layer join");
        }
        // The stroke starts at the first end point and stops at the last (as the editor draws it).
        let mut degree = vec![0usize; net.vertices.len()];
        for s in &net.segments {
            for i in [s.a, s.b] {
                if let Some(d) = degree.get_mut(i) {
                    *d += 1;
                }
            }
        }
        let ends: Vec<usize> = (0..degree.len()).filter(|i| degree.get(*i) == Some(&1)).collect();
        let cap =
            |i: Option<&usize>| i.and_then(|i| net.vertices.get(*i)).and_then(|p| style(p.style)).and_then(|s| s.get("strokeCap")).and_then(Kv::str);
        let (first, last) = (cap(ends.first()), cap(ends.last()));
        if first.is_some() || last.is_some() {
            let node = v.str("strokeCap").unwrap_or("NONE");
            o.insert("strokeCapStart".into(), json!(cap_name(first.unwrap_or(node))));
            o.insert("strokeCapEnd".into(), json!(cap_name(last.unwrap_or(node))));
        }
        // Every instance of a main shows the same network: it is written once and shared.
        let key = (blob_id, sx.to_bits(), sy.to_bits(), std::ptr::from_ref(vd) as usize);
        let id = match self.network_ids.get(&key) {
            Some(id) => *id,
            None => {
                let vertices: Vec<Value> = net
                    .vertices
                    .iter()
                    .map(|p| {
                        let mut out = json!({"x": p.x * sx, "y": p.y * sy});
                        let st = style(p.style);
                        if let Some(r) = st.and_then(|s| num(s, "cornerRadius")).filter(|r| *r > 0.0) {
                            out["radius"] = json!(r);
                        }
                        match st.and_then(|s| s.get("handleMirroring")).and_then(Kv::str) {
                            Some("ANGLE") => out["mirror"] = json!("angle"),
                            Some("ANGLE_AND_LENGTH") => out["mirror"] = json!("angleAndLength"),
                            Some("NONE") => out["mirror"] = json!("none"),
                            _ => {}
                        }
                        if let Some(join @ ("MITER" | "BEVEL" | "ROUND")) = st.and_then(|s| s.get("strokeJoin")).and_then(Kv::str) {
                            out["join"] = json!(join.to_ascii_lowercase());
                        }
                        out
                    })
                    .collect();
                let segments: Vec<Value> = net
                    .segments
                    .iter()
                    .map(|s| json!({"a": s.a, "b": s.b, "ta": [s.ta.0 * sx, s.ta.1 * sy], "tb": [s.tb.0 * sx, s.tb.1 * sy]}))
                    .collect();
                let regions: Vec<Value> =
                    net.regions.iter().map(|r| json!({"loops": r.loops, "winding": if r.non_zero { "nonZero" } else { "evenOdd" }})).collect();
                let mut entry = json!({});
                entry["vertices"] = Value::Array(vertices);
                entry["segments"] = Value::Array(segments);
                entry["regions"] = Value::Array(regions);
                self.networks.push(crate::rest::network(&entry));
                self.network_ids.insert(key, self.networks.len() - 1);
                self.networks.len() - 1
            }
        };
        o.insert("vectorNetworkRef".into(), json!(id));
        true
    }

    /// Component property definitions and description.
    fn component(&mut self, o: &mut Obj, v: &View<'a>, id: &str) {
        let mut defs = Obj::new();
        for d in v.list("componentPropDefs") {
            if d.get("isDeleted").and_then(Kv::bool) == Some(true) {
                continue;
            }
            let Some((name, ty)) = d.get("id").and_then(guid).and_then(|i| self.prop_defs.get(&i).cloned()) else { continue };
            let init = d.get("initialValue");
            let def = match ty {
                "BOOL" => json!({"type": "BOOLEAN", "defaultValue": init.and_then(|i| i.get("boolValue")).and_then(Kv::bool).unwrap_or(true)}),
                "TEXT" => json!({"type": "TEXT", "defaultValue": init.and_then(prop_text).unwrap_or("")}),
                "INSTANCE_SWAP" => match init.and_then(|i| i.get("guidValue")).and_then(guid) {
                    Some(g) => json!({"type": "INSTANCE_SWAP", "defaultValue": g}),
                    None => continue,
                },
                "VARIANT" => json!({"type": "VARIANT"}),
                "SLOT" => {
                    self.warn_non_default_slot_config(d);
                    self.warn_dropped_slot_preferred_values(d);
                    json!({"type": "SLOT"})
                }
                _ => {
                    self.warn(format!("{} component properties are not supported and were skipped", ty.to_ascii_lowercase()));
                    continue;
                }
            };
            defs.insert(name, def);
        }
        if !defs.is_empty() {
            o.insert("componentPropertyDefinitions".into(), Value::Object(defs));
        }
        if let Some(d) = v.str("symbolDescription").or_else(|| v.str("description")).filter(|d| !d.is_empty()) {
            let into = if o.get("type").and_then(Value::as_str) == Some("COMPONENT_SET") { &mut self.sets } else { &mut self.components };
            into.insert(id.to_string(), json!({"description": d}));
        }
    }

    /// Preferred slot values are source component keys, but this mapper has no proven way to map
    /// those keys to file GUIDs. They are dropped instead of guessing native NodeIds.
    fn warn_dropped_slot_preferred_values(&mut self, def: &Kv<'a>) {
        let entries = def.get("preferredValues").and_then(|v| v.get("instanceSwapValues")).map(Kv::list).unwrap_or_default();
        if !entries.is_empty() {
            self.warn("preferred slot component keys cannot be resolved to file nodes and were skipped");
        }
    }

    fn warn_non_default_slot_config(&mut self, def: &Kv<'a>) {
        let Some(config) = def.get("slotPropConfig") else { return };
        let non_default = [
            ("stretchChildOnInsert", Some(false)),
            ("displayByDefault", Some(false)),
            ("minChildren", None),
            ("maxChildren", None),
            ("allowPreferredValuesOnly", Some(false)),
        ]
        .into_iter()
        .any(|(field, bool_default)| {
            let Some(value) = config.get(field) else { return false };
            if let Some(default) = bool_default { value.bool() != Some(default) } else { value.f64() != Some(0.0) }
        });
        if non_default {
            self.warn("non-default slot settings are not represented and were skipped");
        }
    }

    /// A layer's component property references, and inside an instance the values they take.
    fn prop_refs(&mut self, o: &mut Obj, v: &View<'a>, inst: Option<&Inst<'a>>) {
        let mut refs = Obj::new();
        for e in v.get("parameterConsumptionMap").and_then(|m| m.get("entries")).map(Kv::list).unwrap_or_default() {
            let field = match e.get("variableField").and_then(Kv::str) {
                Some("VISIBLE") => "visible",
                Some("TEXT_DATA") => "characters",
                Some("OVERRIDDEN_SYMBOL_ID") => "mainComponent",
                Some("SLOT_CONTENT_ID") => "slot",
                _ => continue,
            };
            let Some(def) =
                e.get("variableData").and_then(|d| d.get("value")).and_then(|x| x.get("propRefValue")).and_then(|p| p.get("defId")).and_then(guid)
            else {
                continue;
            };
            // Expanded frames must follow their main frame's source link. A direct slot reference
            // makes the engine resolve ownership against the instance, where the property is not defined.
            if !(field == "slot" && inst.is_some())
                && let Some((name, _)) = self.prop_defs.get(&def)
            {
                refs.insert(field.into(), json!(name));
            }
            let Some(val) = inst.and_then(|i| i.props.get(&def)) else { continue };
            match field {
                "visible" => {
                    if let Some(b) = val.get("boolValue").and_then(Kv::bool) {
                        o.insert("visible".into(), json!(b));
                    }
                }
                "characters" => {
                    if let Some(t) = prop_text(val) {
                        o.insert("characters".into(), json!(t));
                        // Run styles count the chars they were made for: for other chars only
                        // the value's own apply.
                        if own_text(v) != Some(t) {
                            o.remove("characterStyleOverrides");
                            o.remove("styleOverrideTable");
                            if let Some(tv) = val.get("textValue") {
                                let size = o.get("style").and_then(|s| s.get("fontSize")).and_then(Value::as_f64);
                                char_styles(o, tv, size, &mut |w| self.warn(w));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !refs.is_empty() {
            o.insert("componentPropertyReferences".into(), Value::Object(refs));
        }
    }

    /// An instance's main and properties; the context its expanded layers are built in.
    fn instance(&mut self, o: &mut Obj, v: &View<'a>, id: &str, outer: Option<&Inst<'a>>, rest_id: &str) -> Option<(Inst<'a>, String)> {
        let sd = v.get("symbolData");
        let mut sym = v.get("overriddenSymbolID").and_then(guid).or_else(|| sd.and_then(|s| s.get("symbolID")).and_then(guid));
        // An instance swap property value picks the main.
        if let Some(i) = outer
            && let Some(def) = prop_ref(v, "OVERRIDDEN_SYMBOL_ID")
            && let Some(g) = i.props.get(&def).and_then(|p| p.get("guidValue")).and_then(guid)
        {
            sym = Some(g);
        }
        let Some(sym) = sym else {
            self.warn("instances without a main component were imported as empty frames");
            return None;
        };
        o.insert("componentId".into(), json!(sym));
        let assignments = v.list("componentPropAssignments");
        let mut props = Obj::new();
        for a in assignments {
            let Some((name, ty)) = a.get("defID").and_then(guid).and_then(|d| self.prop_defs.get(&d).cloned()) else { continue };
            let val = a.get("value");
            let p = match ty {
                "BOOL" => val.and_then(|x| x.get("boolValue")).and_then(Kv::bool).map(|b| json!({"type": "BOOLEAN", "value": b})),
                "TEXT" => val.and_then(prop_text).map(|t| json!({"type": "TEXT", "value": t})),
                "INSTANCE_SWAP" => val.and_then(|x| x.get("guidValue")).and_then(guid).map(|g| json!({"type": "INSTANCE_SWAP", "value": g})),
                _ => None,
            };
            if let Some(p) = p {
                props.insert(name, p);
            }
        }
        if !props.is_empty() {
            o.insert("componentProperties".into(), Value::Object(props));
        }
        if !self.nodes.contains_key(&sym) {
            self.warn("instances of components that aren't in the file show no layers");
            return None;
        }
        if outer.is_some_and(|i| i.symbols.contains(&sym)) {
            return None;
        }
        // Overrides by id path below this instance (the instance itself is addressed by its main's id).
        let own_path: Vec<String> = outer.map(|i| [i.path.as_slice(), &[id.to_string()]].concat()).unwrap_or_default();
        let scope = self.override_scope(&sym);
        let mut overrides: Overrides<'a> = BTreeMap::new();
        if let Some(inherited) = outer {
            for (path, rows) in inherited.overrides.iter() {
                let mut path = path.clone();
                // Rows an outer instance wrote for this instance's layers may use their old keys.
                if path.starts_with(&own_path) && path.len() > own_path.len() {
                    normalize_override_segment(&mut path, own_path.len(), &scope);
                }
                overrides.entry(path).or_default().extend(rows.iter().cloned());
            }
        }
        // A nested instance's derived layout is for the size it has in its main: resized here, the
        // sizes and places in it no longer hold and its layers follow their constraints instead.
        let differs = |a: f64, b: f64| (a - b).abs() > 1e-6 * a.abs().max(b.abs()).max(1.0);
        let resized = outer.is_some()
            && match (self.nodes.get(id).and_then(|b| View::of(b).size()), v.size()) {
                (Some(a), Some(b)) => differs(a.0, b.0) || differs(a.1, b.1),
                (a, b) => a.is_some() != b.is_some(),
            };
        let mut local: Overrides<'a> = BTreeMap::new();
        let lists = [sd.and_then(|s| s.get("symbolOverrides")).map(Kv::list).unwrap_or_default(), v.list("derivedSymbolData")];
        let source_depth = outer.map_or(0, |i| i.symbols.len());
        let mut ordinal = 0;
        for (layer, list) in lists.into_iter().enumerate() {
            for ov in list {
                let row_ordinal = ordinal;
                ordinal += 1;
                let mut rel: Vec<String> =
                    ov.get("guidPath").and_then(|g| g.get("guids")).map(Kv::list).unwrap_or_default().iter().filter_map(guid).collect();
                if rel.is_empty() || rel == [sym.clone()] {
                    continue;
                }
                normalize_override_segment(&mut rel, 0, &scope);
                // Explicit overrides, then derived layout (stale once the instance is resized).
                let entry = match (layer, resized) {
                    (0, _) => Entry::Override,
                    (_, false) => Entry::Derived,
                    (_, true) => Entry::Stale,
                };
                local.entry([own_path.as_slice(), &rel].concat()).or_default().push(OverrideRow {
                    row: ov,
                    entry,
                    source_depth,
                    ordinal: row_ordinal,
                });
            }
        }
        for (path, rows) in local {
            overrides.entry(path).or_default().extend(rows);
        }
        for rows in overrides.values_mut() {
            // Deeper source rows apply first; outer rows win, with wire order preserved per source.
            rows.sort_by_key(|row| (std::cmp::Reverse(row.source_depth), row.ordinal));
        }
        let mut all_props: BTreeMap<String, &'a Kv<'a>> = outer.map(|i| (*i.props).clone()).unwrap_or_default();
        let mut slot_contents: BTreeMap<String, String> = outer.map(|i| (*i.slot_contents).clone()).unwrap_or_default();
        for a in assignments {
            if let (Some(d), Some(val)) = (a.get("defID").and_then(guid), a.get("value")) {
                all_props.entry(d).or_insert(val);
            }
            let Some(def) = a.get("defID").and_then(guid) else { continue };
            if self.prop_defs.get(&def).is_none_or(|(_, ty)| *ty != "SLOT") {
                continue;
            }
            let var_value = a.get("varValue");
            if var_value.and_then(|v| v.get("dataType")).and_then(Kv::str) != Some("SLOT_CONTENT_ID") {
                self.warn("malformed slot content assignment was skipped");
                continue;
            }
            let target = var_value.and_then(|v| v.get("value")).and_then(|v| v.get("slotContentIdValue")).and_then(|v| v.get("guid")).and_then(guid);
            let Some(target) = target else {
                self.warn("malformed slot content assignment was skipped");
                continue;
            };
            if target == "4294967295:4294967295" {
                self.warn("slot content assignment uses the empty GUID sentinel and was skipped");
                continue;
            }
            if self.nodes.get(&target).is_none_or(|n| n.get("isSlotContent").and_then(Kv::bool) != Some(true)) {
                self.warn("slot content assignment target is missing or is not slot content");
                continue;
            }
            slot_contents.entry(def).or_insert(target);
        }
        let mut symbols = outer.map(|i| i.symbols.clone()).unwrap_or_default();
        symbols.push(sym.clone());
        let root = outer.map_or_else(|| rest_id.to_string(), |i| i.root.clone());
        Some((
            Inst {
                root,
                path: own_path,
                overrides: Rc::new(overrides),
                props: Rc::new(all_props),
                slot_contents: Rc::new(slot_contents),
                slot_content: false,
                symbols,
            },
            sym,
        ))
    }

    fn prototype(&mut self, o: &mut Obj, v: &View<'a>) {
        let mut list = Vec::new();
        for it in v.list("prototypeInteractions") {
            if it.get("isDeleted").and_then(Kv::bool) == Some(true) {
                continue;
            }
            if let Some(i) = self.interaction(it) {
                list.push(i);
            }
        }
        if !list.is_empty() {
            o.insert("interactions".into(), Value::Array(list));
        }
        let overflow = match v.str("scrollDirection") {
            Some("HORIZONTAL") => Some("HORIZONTAL_SCROLLING"),
            Some("VERTICAL") => Some("VERTICAL_SCROLLING"),
            Some("BOTH") => Some("HORIZONTAL_AND_VERTICAL_SCROLLING"),
            _ => None,
        };
        if let Some(d) = overflow {
            o.insert("overflowDirection".into(), json!(d));
        }
        let behavior = match v.str("scrollBehavior") {
            Some("FIXED_WHEN_CHILD_OF_SCROLLING_FRAME") => Some("FIXED"),
            Some("STICKY_SCROLLS") => Some("STICKY_SCROLLS"),
            _ => None,
        };
        if let Some(b) = behavior {
            o.insert("scrollBehavior".into(), json!(b));
        }
        copy_str(o, v, "overlayPositionType", "overlayPositionType");
        copy_str(o, v, "overlayBackgroundInteraction", "overlayBackgroundInteraction");
        if let Some(a) = v.get("overlayBackgroundAppearance") {
            let ty = a.get("backgroundType").and_then(Kv::str).unwrap_or("NONE");
            o.insert("overlayBackground".into(), json!({"type": ty, "color": a.get("backgroundColor").map_or(Value::Null, color)}));
        }
    }

    fn interaction(&mut self, it: &Kv) -> Option<Value> {
        let ev = it.get("event");
        let ty = match ev.and_then(|e| e.get("interactionType")).and_then(Kv::str).unwrap_or("ON_CLICK") {
            "DRAG" => "ON_DRAG",
            "MOUSE_IN" => "MOUSE_ENTER",
            "MOUSE_OUT" => "MOUSE_LEAVE",
            t => t,
        };
        let mut trigger = json!({"type": ty});
        if let Some(t) = ev.and_then(|e| num(e, "transitionTimeout")) {
            // Seconds as 32-bit floats: whole milliseconds.
            trigger["timeout"] = json!((t * 1000.0).round());
        }
        if let Some(k) = ev.and_then(|e| e.get("keyTrigger")) {
            let codes: Vec<f64> = k.get("keyCodes").map(Kv::list).unwrap_or_default().iter().filter_map(Kv::f64).collect();
            trigger["keyCodes"] = json!(codes);
            if let Some(d) = k.get("triggerDevice").and_then(Kv::str) {
                trigger["device"] = json!(if d.contains("KEYBOARD") { "KEYBOARD" } else { d });
            }
        }
        let actions: Vec<Value> = it.get("actions").map(Kv::list).unwrap_or_default().iter().filter_map(|a| self.action(a, 0)).collect();
        (!actions.is_empty()).then(|| json!({"trigger": trigger, "actions": actions}))
    }

    /// One prototype action in the REST shape (`depth`: conditionals it's nested in).
    fn action(&mut self, a: &Kv, depth: usize) -> Option<Value> {
        Some(match a.get("connectionType").and_then(Kv::str) {
            Some("INTERNAL_NODE") | None => {
                let dest = a.get("transitionNodeID").and_then(guid)?;
                let nav = match a.get("navigationType").and_then(Kv::str) {
                    Some("SWAP_STATE") => "CHANGE_TO",
                    Some(n) => n,
                    None => "NAVIGATE",
                };
                let mut act = json!({"type": "NODE", "destinationId": dest, "navigation": nav});
                if let Some(t) = transition(a) {
                    act["transition"] = t;
                }
                if let Some(b) = a.get("transitionPreserveScroll").and_then(Kv::bool) {
                    act["preserveScrollPosition"] = json!(b);
                }
                if let Some(p) = a.get("overlayRelativePosition") {
                    act["overlayRelativePosition"] = json!({"x": num(p, "x"), "y": num(p, "y")});
                }
                act
            }
            Some("URL") => json!({"type": "URL", "url": a.get("connectionURL").and_then(Kv::str)?}),
            Some("BACK") => json!({"type": "BACK"}),
            Some("CLOSE") => json!({"type": "CLOSE"}),
            Some("NONE") => return None,
            Some("SET_VARIABLE") => {
                let var = a.get("targetVariable").and_then(|t| t.get("id")).and_then(ref_guid).or_else(|| a.get("targetVariableID").and_then(guid));
                // Missing parts are left out: the REST importer skips the action with a warning.
                let mut act = json!({"type": "SET_VARIABLE", "variableId": var});
                if let Some(d) = a.get("targetVariableData").and_then(|d| variable_data(d, 0)) {
                    act["variableValue"] = d;
                }
                act
            }
            Some("SET_VARIABLE_MODE") => json!({
                "type": "SET_VARIABLE_MODE",
                "variableCollectionId": a.get("targetVariableSetID").and_then(ref_guid),
                "variableModeId": a.get("targetVariableModeID").and_then(guid),
            }),
            Some("CONDITIONAL") if depth < MAX_ACTION_DEPTH => {
                let mut blocks = Vec::new();
                for b in a.get("conditionalActions").map(Kv::list).unwrap_or_default() {
                    let actions: Vec<Value> =
                        b.get("actions").map(Kv::list).unwrap_or_default().iter().filter_map(|x| self.action(x, depth + 1)).collect();
                    let mut block = json!({"actions": actions});
                    // A condition that can't be read must not turn the block into an "else".
                    if let Some(c) = b.get("condition") {
                        block["condition"] = variable_data(c, 0).unwrap_or_else(|| json!({}));
                    }
                    blocks.push(block);
                }
                json!({"type": "CONDITIONAL", "conditionalBlocks": blocks})
            }
            Some(other) => {
                self.warn(format!("{} prototype actions are not imported from .fig files yet", other.to_ascii_lowercase().replace('_', " ")));
                return None;
            }
        })
    }

    fn variable_bindings(&mut self, o: &mut Obj, v: &View<'a>) {
        let mut bound = Obj::new();
        let mut text_bound = Obj::new();
        for e in v.get("variableConsumptionMap").and_then(|m| m.get("entries")).map(Kv::list).unwrap_or_default() {
            let Some(var) = alias(e.get("variableData")) else { continue };
            let a = json!({"type": "VARIABLE_ALIAS", "id": var});
            let field = e.get("variableField").and_then(Kv::str).unwrap_or("");
            match (binding_key(field), text_binding_key(field)) {
                (Some(k), _) => {
                    bound.insert(k.into(), a);
                }
                (_, Some(k)) => {
                    text_bound.insert(k.into(), a);
                }
                _ if field == "CORNER_RADIUS" => {
                    for k in ["topLeftRadius", "topRightRadius", "bottomLeftRadius", "bottomRightRadius"] {
                        bound.insert(k.into(), a.clone());
                    }
                }
                _ if field.starts_with("BORDER_") => {
                    let side = match field {
                        "BORDER_TOP_WEIGHT" => "strokeTopWeight",
                        "BORDER_RIGHT_WEIGHT" => "strokeRightWeight",
                        "BORDER_BOTTOM_WEIGHT" => "strokeBottomWeight",
                        _ => "strokeLeftWeight",
                    };
                    bound.insert(side.into(), a);
                }
                _ if field == "WIDTH" || field == "HEIGHT" => {
                    let axis = if field == "WIDTH" { "x" } else { "y" };
                    let size = bound.entry("size").or_insert_with(|| json!({}));
                    size[axis] = a;
                }
                _ => self.warn(format!("variables bound to {} are not imported from .fig files yet", field.to_ascii_lowercase().replace('_', " "))),
            }
        }
        if !bound.is_empty() {
            o.insert("boundVariables".into(), Value::Object(bound));
        }
        // (The text style itself is added after this: it keeps these.)
        if !text_bound.is_empty() {
            o.insert("style".into(), json!({"boundVariables": text_bound}));
        }
        let mut modes = Obj::new();
        let mut lost = false;
        for e in v.get("variableModeBySetMap").and_then(|m| m.get("entries")).map(Kv::list).unwrap_or_default() {
            let (Some(set), Some(mode)) = (e.get("variableSetID"), e.get("variableModeID").and_then(guid)) else { continue };
            match self.resource_id(set) {
                Some(id) => {
                    modes.insert(id, json!(mode));
                }
                // A mode of a collection neither named by guid nor resolvable in the file.
                None if set.get("guid").is_some() || set.get("assetRef").is_some() => lost = true,
                None => {}
            }
        }
        if lost {
            self.warn("variable modes of collections that aren't in the file were skipped");
        }
        if !modes.is_empty() {
            o.insert("explicitVariableModes".into(), Value::Object(modes));
        }
    }

    /// The shared styles the file defines, in their order: each with the sample layer that holds
    /// its values (`styleDefinitions`, our extension of the file JSON; the REST importer reads it).
    fn style_definitions(&mut self) -> Vec<Value> {
        let mut found: Vec<(&'a str, String, &'a Kv<'a>)> = Vec::new();
        for (id, n) in &self.nodes {
            let live = n.get("isSoftDeleted").and_then(Kv::bool) != Some(true) && n.get("isSoftDeletedStyle").and_then(Kv::bool) != Some(true);
            if live && n.get("styleType").and_then(Kv::str).is_some_and(|t| matches!(t, "FILL" | "TEXT" | "EFFECT" | "GRID")) {
                found.push((n.get("sortPosition").and_then(Kv::str).unwrap_or(""), id.clone(), n));
            }
        }
        found.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        let mut out = Vec::new();
        for (_, id, n) in found {
            let Some(node) = self.layer(&id, None, Affine::IDENTITY, None, None, 0) else { break };
            let name = n.get("name").and_then(Kv::str).unwrap_or("Style");
            let desc = n.get("styleDescription").or_else(|| n.get("description")).and_then(Kv::str).unwrap_or("");
            let ty = n.get("styleType").and_then(Kv::str).unwrap_or("");
            out.push(json!({"id": id, "name": name, "description": desc, "styleType": ty, "node": node}));
        }
        out
    }

    /// Collections and variables, as the local variables response's `meta`, with their ids in the
    /// file's order (`collectionOrder` and `variableOrder`, our extensions).
    fn variables(&self) -> Option<Value> {
        let of_type = |t: &'static str, key: &'static str| {
            let live = move |n: &Kv| n.get("type").and_then(Kv::str) == Some(t) && n.get("isSoftDeleted").and_then(Kv::bool) != Some(true);
            let mut list: Vec<(&str, &String, &'a Kv<'a>)> = self
                .nodes
                .iter()
                .filter(|(_, n)| live(n))
                .map(|(id, n)| (n.get(key).and_then(|p| p.get("position").or(Some(p))).and_then(Kv::str).unwrap_or(""), id, *n))
                .collect();
            list.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
            list.into_iter().map(|(_, id, n)| (id, n))
        };
        let mut order = Vec::new();
        let mut collections = Obj::new();
        for (id, n) in of_type("VARIABLE_SET", "parentIndex") {
            order.push(json!(id));
            let mut modes: Vec<(&str, Value)> = n
                .get("variableSetModes")
                .map(Kv::list)
                .unwrap_or_default()
                .iter()
                .filter_map(|m| {
                    let mode = json!({"modeId": m.get("id").and_then(guid)?, "name": m.get("name").and_then(Kv::str).unwrap_or("Mode")});
                    Some((m.get("sortPosition").and_then(Kv::str).unwrap_or(""), mode))
                })
                .collect();
            modes.sort_by(|a, b| a.0.cmp(b.0));
            let default = modes.first().map_or(Value::Null, |m| m.1["modeId"].clone());
            let name = n.get("name").and_then(Kv::str).unwrap_or("Collection");
            let modes: Vec<Value> = modes.into_iter().map(|m| m.1).collect();
            collections.insert(id.clone(), json!({"id": id, "name": name, "modes": modes, "defaultModeId": default}));
        }
        let mut variable_order = Vec::new();
        let mut variables = Obj::new();
        for (id, n) in of_type("VARIABLE", "sortPosition") {
            let Some(set) = n.get("variableSetID").and_then(|s| self.resource_id(s)).filter(|s| collections.contains_key(s)) else { continue };
            let ty = n.get("variableResolvedType").and_then(Kv::str).unwrap_or("");
            if !matches!(ty, "BOOLEAN" | "FLOAT" | "STRING" | "COLOR") {
                continue;
            }
            let mut values = Obj::new();
            for e in n.get("variableDataValues").and_then(|m| m.get("entries")).map(Kv::list).unwrap_or_default() {
                if let (Some(mode), Some(val)) = (e.get("modeID").and_then(guid), e.get("variableData").and_then(variable_value)) {
                    values.insert(mode, val);
                }
            }
            let name = n.get("name").and_then(Kv::str).unwrap_or("Variable");
            let desc = n.get("description").and_then(Kv::str).unwrap_or("");
            let mut var =
                json!({"id": id, "name": name, "variableCollectionId": set, "resolvedType": ty, "valuesByMode": values, "description": desc});
            // The properties pickers offer the variable for, under the REST names.
            if let Some(scopes) = n.get("variableScopes") {
                var["scopes"] = scopes.list().iter().filter_map(Kv::str).map(|s| if s == "STROKE" { "STROKE_COLOR" } else { s }).collect();
            }
            variable_order.push(json!(id));
            variables.insert(id.clone(), var);
        }
        (!collections.is_empty())
            .then(|| json!({"variableCollections": collections, "variables": variables, "collectionOrder": order, "variableOrder": variable_order}))
    }
}

/// The REST `boundVariables` key of a bound layer field.
fn binding_key(field: &str) -> Option<&'static str> {
    Some(match field {
        "STROKE_WEIGHT" => "strokeWeight",
        "STACK_SPACING" => "itemSpacing",
        "STACK_COUNTER_SPACING" => "counterAxisSpacing",
        "STACK_PADDING_LEFT" => "paddingLeft",
        "STACK_PADDING_TOP" => "paddingTop",
        "STACK_PADDING_RIGHT" => "paddingRight",
        "STACK_PADDING_BOTTOM" => "paddingBottom",
        "VISIBLE" => "visible",
        "TEXT_DATA" => "characters",
        "OPACITY" => "opacity",
        "MIN_WIDTH" => "minWidth",
        "MAX_WIDTH" => "maxWidth",
        "MIN_HEIGHT" => "minHeight",
        "MAX_HEIGHT" => "maxHeight",
        "RECTANGLE_TOP_LEFT_CORNER_RADIUS" => "topLeftRadius",
        "RECTANGLE_TOP_RIGHT_CORNER_RADIUS" => "topRightRadius",
        "RECTANGLE_BOTTOM_LEFT_CORNER_RADIUS" => "bottomLeftRadius",
        "RECTANGLE_BOTTOM_RIGHT_CORNER_RADIUS" => "bottomRightRadius",
        _ => return None,
    })
}

/// The REST text style `boundVariables` key of a bound text field.
fn text_binding_key(field: &str) -> Option<&'static str> {
    Some(match field {
        "FONT_SIZE" => "fontSize",
        "FONT_FAMILY" => "fontFamily",
        "FONT_STYLE" => "fontStyle",
        "LETTER_SPACING" => "letterSpacing",
        "LINE_HEIGHT" => "lineHeight",
        "PARAGRAPH_SPACING" => "paragraphSpacing",
        "PARAGRAPH_INDENT" => "paragraphIndent",
        _ => return None,
    })
}

fn stroke(o: &mut Obj, v: &View) {
    copy_num(o, v, "strokeWeight", "strokeWeight");
    if let Some(a) = v.str("strokeAlign").filter(|a| *a != "OFFSET") {
        o.insert("strokeAlign".into(), json!(a));
    }
    copy_str(o, v, "strokeJoin", "strokeJoin");
    if let Some(cap) = v.str("strokeCap") {
        o.insert("strokeCap".into(), json!(cap_name(cap)));
    }
    // A layer written as the outline of its stroke keeps what it was as plugin data
    // (`outlinedStroke`, our extension of the file JSON).
    let (id, key) = super::export::STASH;
    let kept = v.list("pluginData").iter().find(|p| p.get("pluginID").and_then(Kv::str) == Some(id) && p.get("key").and_then(Kv::str) == Some(key));
    if let Some(original) = kept.and_then(|p| p.get("value")).and_then(Kv::str).and_then(|s| serde_json::from_str::<Value>(s).ok()) {
        o.insert("outlinedStroke".into(), original);
    }
    // A variable width: each point's reach on the two sides of the path (shares of the stroke
    // weight), as `[t, scale, bias]` (`strokeWidthProfile`, our extension of the file JSON).
    let profile: Vec<Value> = v
        .list("variableWidthPoints")
        .iter()
        .filter_map(|p| {
            let (left, right) = (num(p, "ascent")?.max(0.0), num(p, "descent")?.max(0.0));
            let bias = if left + right > 0.0 { (right - left) / (left + right) } else { 0.0 };
            Some(json!([num(p, "position")?, left + right, bias]))
        })
        .collect();
    if !profile.is_empty() {
        o.insert("strokeWidthProfile".into(), Value::Array(profile));
    }
    let dashes: Vec<f64> = v.list("dashPattern").iter().filter_map(Kv::f64).collect();
    if !dashes.is_empty() {
        o.insert("strokeDashes".into(), json!(dashes));
    }
    // A miter limit L bevels joins sharper than 2·asin(1/L).
    if let Some(l) = v.num("miterLimit").filter(|l| *l > 1.0) {
        o.insert("strokeMiterAngle".into(), json!((2.0 * (1.0 / l).asin()).to_degrees()));
    }
    if v.flag("borderStrokeWeightsIndependent") == Some(true) {
        // A side that isn't written has no border.
        let side = |k: &str| v.num(k).unwrap_or(0.0);
        o.insert(
            "individualStrokeWeights".into(),
            json!({"top": side("borderTopWeight"), "right": side("borderRightWeight"), "bottom": side("borderBottomWeight"), "left": side("borderLeftWeight")}),
        );
    }
}

/// A `VariableData` (literal, variable reference or expression) in the REST shape; `None` for
/// kinds a prototype can't evaluate.
fn variable_data(d: &Kv, depth: usize) -> Option<Value> {
    let v = d.get("value")?;
    let ty = d.get("dataType").and_then(Kv::str)?;
    let value = match ty {
        "ALIAS" => json!({"type": "VARIABLE_ALIAS", "id": v.get("alias").and_then(ref_guid)?}),
        "EXPRESSION" if depth < MAX_EXPR_DEPTH => {
            let e = v.get("expressionValue")?;
            let function = match e.get("expressionFunction").and_then(Kv::str)? {
                "MULTIPLY" => "MULTIPLICATION",
                "DIVIDE" => "DIVISION",
                f => f,
            };
            let args = e.get("expressionArguments").map(Kv::list).unwrap_or_default();
            let args = args.iter().map(|x| variable_data(x, depth + 1)).collect::<Option<Vec<Value>>>()?;
            json!({"expressionFunction": function, "expressionArguments": args})
        }
        "BOOLEAN" => json!(v.get("boolValue").and_then(Kv::bool)?),
        "FLOAT" => json!(num(v, "floatValue").filter(|x| x.is_finite())?),
        "STRING" => json!(v.get("textValue").and_then(Kv::str)?),
        "COLOR" => color(v.get("colorValue")?),
        _ => return None,
    };
    let mut out = json!({"type": ty, "value": value});
    if let Some(r) = d.get("resolvedDataType").and_then(Kv::str) {
        out["resolvedType"] = json!(r);
    }
    Some(out)
}

/// The text a layer that isn't a text layer carries (a note's, a shape's), unless blank.
fn own_text<'a>(v: &View<'a>) -> Option<&'a str> {
    v.get("textData").and_then(|t| t.get("characters")).and_then(Kv::str).filter(|c| !c.trim().is_empty())
}

/// A generated connector child is identified by one exact template GUID, not its array position.
fn connector_record<'a>(v: &View<'a>, field: &str, local_id: u32) -> Option<&'a Kv<'a>> {
    v.get(field)?.get("overrides")?.list().iter().find(|row| {
        row.get("guidPath").and_then(|path| path.get("guids")).map(Kv::list).is_some_and(|guids| {
            guids.len() == 1 && num(&guids[0], "sessionID") == Some(40_000_000.0) && num(&guids[0], "localID") == Some(f64::from(local_id))
        })
    })
}

fn connector_has_saved_label(v: &View<'_>) -> bool {
    own_text(v).is_some()
        || connector_record(v, "nodeGenerationData", 1)
            .and_then(|r| r.get("textData"))
            .and_then(|d| d.get("characters"))
            .and_then(Kv::str)
            .is_some_and(|s| !s.trim().is_empty())
}

fn valid_size(value: Option<&Kv<'_>>) -> Option<(f64, f64)> {
    let x = num(value?, "x")?;
    let y = num(value?, "y")?;
    (x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0).then_some((x, y))
}

/// The REST name of a stroke cap.
fn cap_name(cap: &str) -> &str {
    match cap {
        "ARROW_LINES" => "LINE_ARROW",
        "ARROW_EQUILATERAL" => "TRIANGLE_ARROW",
        c => c,
    }
}

/// The layer kind (and how the warning names it) that a layer the editor has no counterpart for
/// imports as: frames for notes, shapes, tables, slides and the like (their text as a text layer),
/// vectors for connectors and marker strokes, plain text for text on a path.
fn stand_in(ty: &str) -> Option<(&'static str, &'static str)> {
    Some(match ty {
        "STICKY"
        | "SHAPE_WITH_TEXT"
        | "CODE_BLOCK"
        | "TABLE"
        | "TABLE_CELL"
        | "STAMP"
        | "MEDIA"
        | "WIDGET"
        | "SLIDE"
        | "SLIDE_GRID"
        | "SLIDE_ROW"
        | "INTERACTIVE_SLIDE_ELEMENT"
        | "MODULE" => ("FRAME", "frames with their text"),
        "CONNECTOR" | "HIGHLIGHT" | "WASHI_TAPE" => ("VECTOR", "vectors"),
        "TEXT_PATH" => ("TEXT", "text off its path"),
        _ => return None,
    })
}

/// How a layer sizes and sits in an auto layout parent.
fn child_layout(o: &mut Obj, v: &View, parent: Option<&View>) {
    copy_num(o, v, "stackChildPrimaryGrow", "layoutGrow");
    if v.str("stackChildAlignSelf") == Some("STRETCH") {
        o.insert("layoutAlign".into(), json!("STRETCH"));
    }
    if v.str("stackPositioning") == Some("ABSOLUTE") {
        o.insert("layoutPositioning".into(), json!("ABSOLUTE"));
    }
    for (k, w, h) in [("minSize", "minWidth", "minHeight"), ("maxSize", "maxWidth", "maxHeight")] {
        if let Some(val) = v.get(k).and_then(|s| s.get("value")) {
            for (axis, to) in [("x", w), ("y", h)] {
                if let Some(x) = num(val, axis).filter(|x| *x > 0.0 && *x < 1e9) {
                    o.insert(to.into(), json!(x));
                }
            }
        }
    }
    if let Some(p) = parent.filter(|p| p.str("stackMode") == Some("GRID")) {
        for (anchor, list, to) in [("gridColumnAnchor", "gridColumns", "gridColumnAnchorIndex"), ("gridRowAnchor", "gridRows", "gridRowAnchorIndex")]
        {
            if let Some(i) = v.get(anchor).and_then(guid).and_then(|g| tracks(p, list).iter().position(|t| *t == g)) {
                o.insert(to.into(), json!(i));
            }
        }
        copy_num(o, v, "gridColumnSpan", "gridColumnSpan");
        copy_num(o, v, "gridRowSpan", "gridRowSpan");
    }
}

/// Whether a paint's colour or one of its gradient stops is bound to a variable.
fn paint_bound(p: &Kv) -> bool {
    alias(p.get("colorVar")).is_some() || p.get("stopsVar").map(Kv::list).unwrap_or_default().iter().any(|s| alias(s.get("colorVar")).is_some())
}

fn paint(p: &Kv, warn: &mut dyn FnMut(String)) -> Option<Value> {
    let ty = p.get("type").and_then(Kv::str)?;
    let mut o = Obj::new();
    o.insert("type".into(), json!(ty));
    if let Some(x) = num(p, "opacity") {
        o.insert("opacity".into(), json!(x));
    }
    if let Some(b) = p.get("visible").and_then(Kv::bool) {
        o.insert("visible".into(), json!(b));
    }
    if let Some(b) = p.get("blendMode").and_then(Kv::str) {
        o.insert("blendMode".into(), json!(b));
    }
    let t = p.get("transform").and_then(matrix);
    match ty {
        "SOLID" => {
            if let Some(c) = p.get("color") {
                o.insert("color".into(), color(c));
            }
            if let Some(var) = alias(p.get("colorVar")) {
                o.insert("boundVariables".into(), json!({"color": {"type": "VARIABLE_ALIAS", "id": var}}));
            }
        }
        _ if ty.starts_with("GRADIENT_") => {
            // The paint transform maps the layer's unit box to gradient space, where a linear
            // gradient runs from (0, ½) to (1, ½) and the others are centred on (½, ½).
            let inv = t.filter(|t| t.determinant().abs() > 1e-12).map_or(Affine::IDENTITY, Affine::inverse);
            let pts = if ty == "GRADIENT_LINEAR" { [(0.0, 0.5), (1.0, 0.5), (0.0, 1.0)] } else { [(0.5, 0.5), (1.0, 0.5), (0.5, 1.0)] };
            let handles: Vec<Value> = pts
                .iter()
                .map(|(x, y)| {
                    let q = inv * Point::new(*x, *y);
                    json!({"x": q.x, "y": q.y})
                })
                .collect();
            o.insert("gradientHandlePositions".into(), Value::Array(handles));
            // Stops bound to variables are listed again, each with its variable.
            let vars = p.get("stopsVar").map(Kv::list).unwrap_or_default();
            let stops: Vec<Value> = p
                .get("stops")
                .map(Kv::list)
                .unwrap_or_default()
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let mut stop = json!({"position": num(s, "position").unwrap_or(0.0), "color": s.get("color").map_or(Value::Null, color)});
                    if let Some(b) = bound(vars.get(i), &[("colorVar", "color")]) {
                        stop["boundVariables"] = b;
                    }
                    stop
                })
                .collect();
            o.insert("gradientStops".into(), Value::Array(stops));
        }
        "IMAGE" => {
            let hash: String =
                p.get("image").and_then(|i| i.get("hash")).and_then(Kv::bytes).unwrap_or_default().iter().map(|b| format!("{b:02x}")).collect();
            o.insert("imageRef".into(), json!(hash));
            let mode = p.get("imageScaleMode").and_then(Kv::str).unwrap_or("FILL");
            o.insert("scaleMode".into(), json!(mode));
            if let (Some(t), "STRETCH") = (t, mode) {
                o.insert("imageTransform".into(), rows(t));
            }
            if let Some(s) = num(p, "scale") {
                o.insert("scalingFactor".into(), json!(s));
            }
            if let Some(r) = num(p, "rotation") {
                o.insert("rotation".into(), json!(r));
            }
            match (p.get("filterColorAdjust"), p.get("paintFilter")) {
                (Some(_), Some(_)) => {
                    warn("image filters are ambiguous because both filter containers are present; filter conversion was skipped".into())
                }
                (Some(_), None) => warn("legacy image filter container is unsupported; filter conversion was skipped".into()),
                (None, Some(filters)) => {
                    let mut mapped = Obj::new();
                    let fields = [
                        ("exposure", "exposure"),
                        ("contrast", "contrast"),
                        ("vibrance", "saturation"),
                        ("temperature", "temperature"),
                        ("tint", "tint"),
                        ("highlights", "highlights"),
                        ("shadows", "shadows"),
                    ];
                    if let Kv::Object(values) = filters {
                        for (source, target) in fields {
                            if let Some(value) = filters.get(source) {
                                match filter_number(value) {
                                    Some(value) => {
                                        mapped.insert(target.into(), json!(value));
                                    }
                                    None => warn(format!("malformed image filter `{source}` was skipped")),
                                }
                            }
                        }
                        for (name, value) in values {
                            if fields.iter().any(|(known, _)| *known == *name) {
                                continue;
                            }
                            match filter_number(value) {
                                Some(0.0) => {}
                                Some(_) => warn(format!("nonzero unsupported image filter `{}` was skipped", crate::rest::props::clip_str(name))),
                                None => warn(format!("malformed unsupported image filter `{}` was skipped", crate::rest::props::clip_str(name))),
                            }
                        }
                    } else {
                        warn("malformed image paintFilter container was skipped".into());
                    }
                    if !mapped.is_empty() {
                        o.insert("filters".into(), Value::Object(mapped));
                    }
                }
                (None, None) => {}
            }
        }
        _ => {}
    }
    Some(Value::Object(o))
}

/// The variables bound to the fields of a paint stop, an effect or a layout grid, as its REST
/// `boundVariables`: `fields` pairs the format's field with the REST name.
fn bound(v: Option<&Kv>, fields: &[(&str, &str)]) -> Option<Value> {
    let v = v?;
    let found: Obj =
        fields.iter().filter_map(|(from, to)| Some((to.to_string(), json!({"type": "VARIABLE_ALIAS", "id": alias(v.get(from))?})))).collect();
    (!found.is_empty()).then_some(Value::Object(found))
}

/// The track ids of a grid's columns or rows, in order.
fn tracks(v: &View, key: &str) -> Vec<String> {
    let mut t: Vec<(&str, String)> = v
        .get(key)
        .and_then(|m| m.get("entries"))
        .map(Kv::list)
        .unwrap_or_default()
        .iter()
        .filter_map(|e| Some((e.get("position").and_then(Kv::str).unwrap_or(""), e.get("id").and_then(guid)?)))
        .collect();
    t.sort();
    t.into_iter().map(|e| e.1).collect()
}

/// Convert only unconstrained tracks, whose min and max sizing functions agree.
fn grid_track_sizing(size: Option<&Kv<'_>>) -> Option<Value> {
    let size = size?;
    let min = size.get("minSizing")?;
    let max = size.get("maxSizing")?;
    let min_type = min.get("type").and_then(Kv::str)?;
    let max_type = max.get("type").and_then(Kv::str)?;
    if min_type != max_type {
        return None;
    }
    match min_type {
        "FIXED" => {
            let px = num(min, "value")?;
            let max_px = num(max, "value")?;
            (px == max_px && px.is_finite() && (0.0..=MAX_GRID_TRACK_VALUE).contains(&px)).then(|| json!({"sizing": "fixed", "value": px}))
        }
        "FLEX" => {
            let fr = num(min, "value")?;
            let max_fr = num(max, "value")?;
            (fr == max_fr && fr.is_finite() && fr > 0.0 && fr <= MAX_GRID_TRACK_VALUE).then(|| json!({"sizing": "fill", "value": fr}))
        }
        "HUG" => Some(json!({"sizing": "hug", "value": 0.0})),
        _ => None,
    }
}

/// The variable an `ALIAS` variable data points at.
fn alias(d: Option<&Kv>) -> Option<String> {
    let d = d?;
    if d.get("dataType").and_then(Kv::str) != Some("ALIAS") {
        return None;
    }
    d.get("value").and_then(|v| v.get("alias")).and_then(ref_guid)
}

fn variable_value(d: &Kv) -> Option<Value> {
    if let Some(a) = alias(Some(d)) {
        return Some(json!({"type": "VARIABLE_ALIAS", "id": a}));
    }
    let v = d.get("value")?;
    if let Some(c) = v.get("colorValue") {
        return Some(color(c));
    }
    if let Some(b) = v.get("boolValue").and_then(Kv::bool) {
        return Some(json!(b));
    }
    if let Some(x) = num(v, "floatValue") {
        return Some(json!(x));
    }
    v.get("textValue").and_then(Kv::str).map(|s| json!(s))
}

/// Per-character styles of text data `td` (`characterStyleIDs` + `styleOverrideTable`) as REST
/// `characterStyleOverrides` + `styleOverrideTable`; `size`: the base font size.
fn char_styles(o: &mut Obj, td: &Kv, size: Option<f64>, warn: &mut dyn FnMut(String)) {
    let ids: Vec<u64> = td.get("characterStyleIDs").map(Kv::list).unwrap_or_default().iter().filter_map(Kv::f64).map(|i| i as u64).collect();
    if ids.iter().all(|i| *i == 0) {
        return;
    }
    let mut table = Obj::new();
    for e in td.get("styleOverrideTable").map(Kv::list).unwrap_or_default() {
        let Some(sid) = num(e, "styleID") else { continue };
        let mut s = text_style(&View::of(e), size);
        if let Some(fills) = e.get("fillPaints") {
            s.insert("fills".into(), Value::Array(fills.list().iter().filter_map(|p| paint(p, warn)).collect()));
        }
        table.insert((sid as u64).to_string(), Value::Object(s));
    }
    o.insert("characterStyleOverrides".into(), json!(ids));
    o.insert("styleOverrideTable".into(), Value::Object(table));
}

/// The text of a component property value.
fn prop_text<'k>(v: &'k Kv) -> Option<&'k str> {
    v.get("textValue").and_then(|t| t.get("characters")).and_then(Kv::str)
}

/// The property a layer's `field` is bound to (`parameterConsumptionMap`).
fn prop_ref(v: &View, field: &str) -> Option<String> {
    v.get("parameterConsumptionMap")?.get("entries")?.list().iter().find_map(|e| {
        if e.get("variableField").and_then(Kv::str) != Some(field) {
            return None;
        }
        e.get("variableData")?.get("value")?.get("propRefValue")?.get("defId").and_then(guid)
    })
}

/// The REST TypeStyle fields a node (or text override entry) sets. `size`: the font size that
/// letter spacing in percent refers to when the entry doesn't set one.
fn text_style(v: &View, size: Option<f64>) -> Obj {
    let mut s = Obj::new();
    if let Some(f) = v.get("fontName") {
        for (from, to) in [("family", "fontFamily"), ("style", "fontStyle"), ("postscript", "fontPostScriptName")] {
            if let Some(x) = f.get(from).and_then(Kv::str).filter(|x| !x.is_empty()) {
                // Compact and spaced style names ("SemiBold", "Semi Bold") are the same style.
                s.insert(to.into(), if from == "style" { json!(super::style_name(x)) } else { json!(x) });
            }
        }
    }
    copy_num(&mut s, v, "fontSize", "fontSize");
    copy_num(&mut s, v, "paragraphSpacing", "paragraphSpacing");
    copy_num(&mut s, v, "paragraphIndent", "paragraphIndent");
    insert_text_options(&mut s, v);
    copy_str(&mut s, v, "textCase", "textCase");
    copy_str(&mut s, v, "textDecoration", "textDecoration");
    if let Some(skip_ink) = v.flag("textDecorationSkipInk") {
        s.insert("textDecorationSkipInk".into(), json!(skip_ink));
    }
    copy_str(&mut s, v, "leadingTrim", "leadingTrim");
    let font_size = v.num("fontSize").or(size).unwrap_or(12.0);
    if let Some(ls) = v.get("letterSpacing") {
        let val = num(ls, "value").unwrap_or(0.0);
        let px = match ls.get("units").and_then(Kv::str) {
            Some("PIXELS") => val,
            Some("PERCENT") => {
                // Private (read by `rest/text.rs`): REST spacing is in pixels; keep the percent.
                s.insert("_letterSpacingPercent".into(), json!(val));
                val / 100.0 * font_size
            }
            _ => val * font_size,
        };
        s.insert("letterSpacing".into(), json!(px));
    }
    if let Some(lh) = v.get("lineHeight") {
        let val = num(lh, "value").unwrap_or(100.0);
        match lh.get("units").and_then(Kv::str) {
            Some("PIXELS") => {
                s.insert("lineHeightUnit".into(), json!("PIXELS"));
                s.insert("lineHeightPx".into(), json!(val));
            }
            // shortcut: percent units count from the font's own line height (100 % is "auto"), which
            // the document can only say for 100 %; other values read as % of the font size. Upgrade
            // when line heights can be a share of the font's own.
            Some("PERCENT") if (val - 100.0).abs() < 1e-6 => {
                s.insert("lineHeightUnit".into(), json!("INTRINSIC_%"));
            }
            Some("PERCENT") => {
                s.insert("lineHeightUnit".into(), json!("FONT_SIZE_%"));
                s.insert("lineHeightPercentFontSize".into(), json!(val));
            }
            _ => {
                s.insert("lineHeightUnit".into(), json!("FONT_SIZE_%"));
                s.insert("lineHeightPercentFontSize".into(), json!(val * 100.0));
            }
        }
    }
    // An empty URL is kept: on an override entry it clears the text's own link.
    if let Some(u) = v.get("hyperlink").and_then(|h| h.get("url")).and_then(Kv::str) {
        s.insert("hyperlink".into(), json!({"type": "URL", "url": u}));
    }
    s
}

fn effect(e: &Kv) -> Option<Value> {
    let ty = match e.get("type").and_then(Kv::str)? {
        "FOREGROUND_BLUR" => "LAYER_BLUR",
        "GRAIN" => "TEXTURE",
        t => t,
    };
    let mut o = Obj::new();
    o.insert("type".into(), json!(ty));
    for k in ["radius", "spread", "startRadius", "density", "opacity"] {
        if let Some(x) = num(e, k) {
            o.insert(k.into(), json!(x));
        }
    }
    for k in ["visible", "showShadowBehindNode", "clipToShape"] {
        if let Some(b) = e.get(k).and_then(Kv::bool) {
            o.insert(k.into(), json!(b));
        }
    }
    for k in ["blendMode", "noiseType"] {
        if let Some(s) = e.get(k).and_then(Kv::str) {
            o.insert(k.into(), json!(s));
        }
    }
    for k in ["color", "secondaryColor"] {
        if let Some(c) = e.get(k) {
            o.insert(k.into(), color(c));
        }
    }
    for (from, to) in [("offset", "offset"), ("startOffset", "startOffset"), ("endOffset", "endOffset"), ("noiseSize", "noiseSizeVector")] {
        if let Some(p) = e.get(from).and_then(vec2) {
            o.insert(to.into(), p);
        }
    }
    if e.get("blurOpType").and_then(Kv::str) == Some("PROGRESSIVE") {
        o.insert("blurType".into(), json!("PROGRESSIVE"));
    }
    let vars = [
        ("colorVar", "color"),
        ("radiusVar", "radius"),
        ("spreadVar", "spread"),
        ("xVar", "offsetX"),
        ("yVar", "offsetY"),
        ("startRadiusVar", "startRadius"),
    ];
    if let Some(b) = bound(Some(e), &vars) {
        o.insert("boundVariables".into(), b);
    }
    Some(Value::Object(o))
}

fn layout_grid(g: &Kv) -> Value {
    let pattern = match (g.get("pattern").and_then(Kv::str), g.get("axis").and_then(Kv::str)) {
        (Some("GRID"), _) => "GRID",
        (_, Some("Y")) => "ROWS",
        _ => "COLUMNS",
    };
    let mut o = json!({"pattern": pattern, "alignment": g.get("type").and_then(Kv::str).unwrap_or("STRETCH")});
    for (from, to) in [("sectionSize", "sectionSize"), ("numSections", "count"), ("gutterSize", "gutterSize"), ("offset", "offset")] {
        if let Some(x) = num(g, from) {
            o[to] = json!(x);
        }
    }
    if let Some(c) = g.get("color") {
        o["color"] = color(c);
    }
    if let Some(b) = g.get("visible").and_then(Kv::bool) {
        o["visible"] = json!(b);
    }
    let vars = [("numSectionsVar", "count"), ("offsetVar", "offset"), ("sectionSizeVar", "sectionSize"), ("gutterSizeVar", "gutterSize")];
    if let Some(b) = bound(Some(g), &vars) {
        o["boundVariables"] = b;
    }
    o
}

fn export_setting(e: &Kv) -> Value {
    let c = e.get("constraint");
    let ty = match c.and_then(|c| c.get("type")).and_then(Kv::str) {
        Some("CONTENT_WIDTH") => "WIDTH",
        Some("CONTENT_HEIGHT") => "HEIGHT",
        _ => "SCALE",
    };
    json!({
        "format": e.get("imageType").and_then(Kv::str).unwrap_or("PNG"),
        "suffix": e.get("suffix").and_then(Kv::str).unwrap_or(""),
        "constraint": {"type": ty, "value": c.and_then(|c| num(c, "value")).unwrap_or(1.0)},
    })
}

/// A prototype action's transition (`None`: instant).
fn transition(a: &Kv) -> Option<Value> {
    let ty = a.get("transitionType").and_then(Kv::str)?;
    // Directions name where the incoming screen moves (SLIDE_FROM_LEFT moves right).
    let (kind, dir) = match ty {
        "INSTANT_TRANSITION" => return None,
        "DISSOLVE" | "FADE" => ("DISSOLVE", None),
        "SMART_ANIMATE" | "MAGIC_MOVE" => ("SMART_ANIMATE", None),
        "SCROLL_ANIMATE" => ("SCROLL_ANIMATE", None),
        t => {
            let prefixes = [
                ("SLIDE_FROM_", "SLIDE_IN"),
                ("PUSH_FROM_", "PUSH"),
                ("MOVE_FROM_", "MOVE_IN"),
                ("SLIDE_OUT_TO_", "SLIDE_OUT"),
                ("MOVE_OUT_TO_", "MOVE_OUT"),
            ];
            let (kind, side) = prefixes.iter().find_map(|(p, k)| t.strip_prefix(p).map(|r| (*k, r)))?;
            let incoming = matches!(kind, "SLIDE_IN" | "PUSH" | "MOVE_IN");
            let dir = match (side, incoming) {
                ("LEFT", true) | ("RIGHT", false) => "RIGHT",
                ("RIGHT", true) | ("LEFT", false) => "LEFT",
                ("TOP", true) | ("BOTTOM", false) => "BOTTOM",
                _ => "TOP",
            };
            (kind, Some(dir))
        }
    };
    let easing = match a.get("easingType").and_then(Kv::str) {
        Some("IN_CUBIC" | "EASE_IN") => "EASE_IN",
        Some("INOUT_CUBIC") => "EASE_IN_AND_OUT",
        Some("LINEAR") => "LINEAR",
        Some("IN_BACK_CUBIC") => "EASE_IN_BACK",
        Some("OUT_BACK_CUBIC") => "EASE_OUT_BACK",
        Some("INOUT_BACK_CUBIC") => "EASE_IN_AND_OUT_BACK",
        Some("GENTLE_SPRING") => "GENTLE",
        Some("CUSTOM_CUBIC") => "CUSTOM_CUBIC_BEZIER",
        _ => "EASE_OUT",
    };
    let mut e = json!({"type": easing});
    let f: Vec<f64> = a.get("easingFunction").map(Kv::list).unwrap_or_default().iter().filter_map(Kv::f64).collect();
    if let (true, [x1, y1, x2, y2, ..]) = (easing == "CUSTOM_CUBIC_BEZIER", f.as_slice()) {
        e["easingFunctionCubicBezier"] = json!({"x1": x1, "y1": y1, "x2": x2, "y2": y2});
    }
    let mut t = json!({"type": kind, "duration": (num(a, "transitionDuration").unwrap_or(0.3) * 1000.0).round(), "easing": e});
    if let Some(d) = dir {
        t["direction"] = json!(d);
    }
    Some(t)
}

/// SVG path data of a geometry blob: commands `0` close, `1` move, `2` line, `3` quadratic and
/// `4` cubic, each followed by its points as little-endian `f32` pairs. `None` when damaged.
fn svg_path(b: &[u8]) -> Option<String> {
    let mut out = String::new();
    let mut i = 0usize;
    let mut n = 0usize;
    while i < b.len() {
        n += 1;
        if n > MAX_COMMANDS {
            return None;
        }
        let (letter, points) = match *b.get(i)? {
            0 => ('Z', 0),
            1 => ('M', 1),
            2 => ('L', 1),
            3 => ('Q', 2),
            4 => ('C', 3),
            _ => return None,
        };
        i += 1;
        if !out.is_empty() {
            out.push(' ');
        }
        out.push(letter);
        for _ in 0..points * 2 {
            let bytes: [u8; 4] = b.get(i..i + 4)?.try_into().ok()?;
            let x = f32::from_le_bytes(bytes);
            if !x.is_finite() {
                return None;
            }
            out.push_str(&format!(" {x}"));
            i += 4;
        }
    }
    Some(out)
}
