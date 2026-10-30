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

mod component;
mod connector;
mod document;
mod layer;
mod layout;
mod paint;
mod proto;
mod text;
mod variables;

use component::OverrideScope;
#[cfg(test)]
use paint::paint;

type Obj = Map<String, Value>;

/// Deepest nesting followed.
const MAX_DEPTH: usize = 256;
/// Most path commands read from one geometry blob.
const MAX_COMMANDS: usize = 200_000;
const MAX_GRID_TRACKS: usize = 1_000;
pub(super) const MAX_GRID_TRACK_VALUE: f64 = 1e6;
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
    // Mains in the file that the layer cap kept out: their instances are told apart from those of
    // components the file doesn't have (#981).
    if m.count >= m.max_nodes {
        let capped: Vec<Value> = m.referenced_mains.difference(&m.mapped_mains).map(|id| json!(id)).collect();
        if !capped.is_empty() {
            out[crate::rest::FIG_CAPPED_MAINS_FIELD] = Value::Array(capped);
        }
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
    (out, m.warnings.finish(), m.networks)
}

#[cfg(test)]
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

/// A generated child is identified by one exact template GUID, not its array position.
fn generated_record<'a>(v: &View<'a>, field: &str, local_id: u32) -> Option<&'a Kv<'a>> {
    v.get(field)?.get("overrides")?.list().iter().find(|row| {
        row.get("guidPath").and_then(|path| path.get("guids")).map(Kv::list).is_some_and(|guids| {
            guids.len() == 1 && num(&guids[0], "sessionID") == Some(40_000_000.0) && num(&guids[0], "localID") == Some(f64::from(local_id))
        })
    })
}

fn num(n: &Kv, k: &str) -> Option<f64> {
    n.get(k).and_then(Kv::f64)
}

fn guid(v: &Kv) -> Option<String> {
    Some(format!("{}:{}", num(v, "sessionID")? as u64, num(v, "localID")? as u64))
}

#[cfg(test)]
mod connector_snapshot_budget_tests;

/// A `GUID`-holding reference (`StyleId`, `VariableID`, …): its `guid`.
fn ref_guid(v: &Kv) -> Option<String> {
    v.get("guid").and_then(guid).filter(|g| g != NO_STYLE)
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

    /// This view with `style` just above the layer that links it in `key` (or `legacy`): the style
    /// beats that layer's and lower layers' copies of its fields (stale when the style changed
    /// since, or written for the style linked before), and override rows above the link beat it.
    fn styled(&self, style: Option<&'a Kv<'a>>, key: &str, legacy: &str) -> Self {
        let mut v = self.clone();
        if let Some(s) = style {
            let linked_in = |k: &str| v.layers.iter().rposition(|l| l.get(k).is_some());
            let at = linked_in(key).or_else(|| linked_in(legacy)).map_or(1, |i| i + 1);
            v.layers.insert(at.min(v.layers.len()), s);
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

struct Mapper<'a> {
    nodes: BTreeMap<String, &'a Kv<'a>>,
    kids: BTreeMap<String, Vec<(&'a str, String)>>,
    blobs: Vec<&'a [u8]>,
    /// Component property definitions: id → (REST name "Name#id", type).
    prop_defs: BTreeMap<String, (String, &'a str)>,
    /// The copies of a set's definitions each variant keeps (no name or type of their own; the
    /// variant's layers and its instances' assignments use them): copy id → the definition's id.
    prop_parents: BTreeMap<String, String>,
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
    warnings: crate::rest::Warnings,
    count: usize,
    /// The mains in the file that instances or instance swap defaults name, and the mains mapped
    /// as layers: the difference is what the layer cap kept out.
    referenced_mains: BTreeSet<String>,
    mapped_mains: BTreeSet<String>,
    /// Most layers produced or scanned, instance expansion included; the one place the cap comes
    /// from (see [`super::layer_budget`]).
    max_nodes: usize,
    /// Layers kept back for the visible pages' top level while library copies are mapped, so the
    /// copies can't take the whole budget and leave the pages empty.
    reserved: usize,
    /// Vector networks, each built once: the layers that show one (every instance of a main
    /// holding it) name it by `vectorNetworkRef`. Kept out of the JSON, where a large file's
    /// networks took gigabytes.
    networks: Vec<Option<VectorNetwork>>,
    /// Index in `networks` by what a network is made from: its blob, its scale and its style table.
    network_ids: HashMap<(usize, u64, u64, usize), usize>,
    /// Region paint elements (paints and gradient stops) read so far: one budget for the whole
    /// file, since layers with region paints don't share their network.
    region_paint_elements: usize,
}

impl<'a> Mapper<'a> {
    fn new(msg: &'a Kv<'a>, max_nodes: usize) -> Self {
        let mut nodes = BTreeMap::new();
        let mut kids: BTreeMap<String, Vec<(&'a str, String)>> = BTreeMap::new();
        let mut prop_defs = BTreeMap::new();
        let mut prop_parents = BTreeMap::new();
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
            // Variables and their collections are read from the node list (`of_type`), never
            // as layers: out of the layer tree, an instance's expansion doesn't walk them again for
            // every instance without counting them (#1077).
            if let Some(p) = n.get("parentIndex")
                && let Some(parent) = p.get("guid").and_then(guid)
                && !matches!(n.get("type").and_then(Kv::str), Some("VARIABLE" | "VARIABLE_SET"))
            {
                kids.entry(parent).or_default().push((p.get("position").and_then(Kv::str).unwrap_or(""), id.clone()));
            }
            for d in n.get("componentPropDefs").map(Kv::list).unwrap_or_default() {
                let Some(did) = d.get("id").and_then(guid) else { continue };
                if let (Some(name), Some(ty)) = (d.get("name").and_then(Kv::str), d.get("type").and_then(Kv::str)) {
                    prop_defs.insert(did.clone(), (format!("{name}#{did}"), ty));
                } else if let Some(parent) = d.get("parentPropDefId").and_then(guid) {
                    prop_parents.insert(did, parent);
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
            prop_parents,
            assets,
            styles: Obj::new(),
            components: Obj::new(),
            sets: Obj::new(),
            override_scopes: BTreeMap::new(),
            override_scope_entries: 0,
            library_contexts: Vec::new(),
            library_scan_count: 0,
            warnings: crate::rest::Warnings::default(),
            count: 0,
            referenced_mains: BTreeSet::new(),
            mapped_mains: BTreeSet::new(),
            max_nodes,
            reserved: 0,
            networks: Vec::new(),
            network_ids: HashMap::new(),
            region_paint_elements: 0,
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

    /// A note about what this file's mapping left out or changed. Counted, as the REST importer
    /// counts its own: these notes are met once per layer, so the kinds were named once each with
    /// no sign of how much of the file they covered -- a file whose every text layer lost a
    /// setting read the same as one where a single layer did (#467). The collector keeps the
    /// first-seen order and the same cap, since some notes name fields of the file (image
    /// filters).
    fn warn(&mut self, w: impl Into<String>) {
        self.warnings.add(w.into());
    }

    fn children(&self, id: &str) -> Vec<String> {
        self.kids.get(id).map(|k| k.iter().map(|(_, c)| c.clone()).collect()).unwrap_or_default()
    }

    /// How many layers may be produced now: the cap, less what is kept for the visible pages.
    fn budget(&self) -> usize {
        self.max_nodes.saturating_sub(self.reserved)
    }
}
