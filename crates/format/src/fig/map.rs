//! Decoded `.fig` message → REST API file JSON, which [`crate::rest`] then imports.
//!
//! The message lists node changes; each names its id (`guid`), its parent and its position among
//! the siblings (`parentIndex`). Nodes are read by field name, so files written with newer
//! schemas still import what is understood. Instances carry only their overrides: their layers
//! are expanded here from the main component (overrides and derived layout matched by id path,
//! component property values applied) so the import looks as saved.

use std::collections::BTreeMap;
use std::rc::Rc;

use figcraft_doc::geom::{Affine, Point};
use figcraft_doc::proto::{MAX_ACTION_DEPTH, MAX_EXPR_DEPTH};
use serde_json::{Map, Value, json};

use super::kiwi::Kv;
use super::network;

type Obj = Map<String, Value>;

/// Most layers produced (instance expansion included) and deepest nesting followed.
const MAX_NODES: usize = crate::rest::MAX_NODES;
const MAX_DEPTH: usize = 256;
/// Most path commands read from one geometry blob.
const MAX_COMMANDS: usize = 200_000;
/// Id of "no style".
const NO_STYLE: &str = "4294967295:4294967295";
/// Longest asset `key` or `version` kept for resolving references given as assets.
const MAX_ASSET_PART: usize = 256;

/// The REST API file JSON for message `msg`, with the warnings about what was skipped.
pub(super) fn to_rest(msg: &Kv, name: &str) -> (Value, Vec<String>) {
    let mut m = Mapper::new(msg);
    let doc = m.document();
    let mut out = json!({"name": name, "document": doc});
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
    (out, m.warnings)
}

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

fn guid(v: &Kv) -> Option<String> {
    Some(format!("{}:{}", num(v, "sessionID")? as u64, num(v, "localID")? as u64))
}

/// A `GUID`-holding reference (`StyleId`, `VariableID`, …): its `guid`.
fn ref_guid(v: &Kv) -> Option<String> {
    v.get("guid").and_then(guid).filter(|g| g != NO_STYLE)
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
type Overrides<'a> = BTreeMap<Vec<String>, Vec<(Entry, &'a Kv<'a>)>>;

/// Where an override entry comes from.
#[derive(Clone, Copy, PartialEq)]
enum Entry {
    Override,
    Derived,
    /// Derived layout of a nested instance for a size it no longer has: its size and place no
    /// longer hold, the rest (resolved colours, text) still does.
    Stale,
}

/// Where an instance's expanded layers are: the overrides and derived layout of the outermost
/// instance (by id path), the path of the instance being expanded, and the property values.
#[derive(Clone)]
struct Inst<'a> {
    root: String,
    path: Vec<String>,
    overrides: Rc<Overrides<'a>>,
    props: Rc<BTreeMap<String, &'a Kv<'a>>>,
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
    warnings: Vec<String>,
    count: usize,
}

impl<'a> Mapper<'a> {
    fn new(msg: &'a Kv<'a>) -> Self {
        let mut nodes = BTreeMap::new();
        let mut kids: BTreeMap<String, Vec<(&'a str, String)>> = BTreeMap::new();
        let mut prop_defs = BTreeMap::new();
        for n in msg.get("nodeChanges").map(Kv::list).unwrap_or_default() {
            let Some(id) = n.get("guid").and_then(guid) else { continue };
            if n.get("phase").and_then(Kv::str) == Some("REMOVED") {
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
            nodes.insert(id, n);
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
        Self { nodes, kids, blobs, prop_defs, assets, styles: Obj::new(), components: Obj::new(), sets: Obj::new(), warnings: Vec::new(), count: 0 }
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
        if !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }

    fn children(&self, id: &str) -> Vec<String> {
        self.kids.get(id).map(|k| k.iter().map(|(_, c)| c.clone()).collect()).unwrap_or_default()
    }

    /// The DOCUMENT with its pages.
    fn document(&mut self) -> Value {
        let doc_id = self.nodes.iter().find(|(_, n)| n.get("type").and_then(Kv::str) == Some("DOCUMENT")).map(|(id, _)| id.clone());
        let pages: Vec<String> = match &doc_id {
            Some(d) => self.children(d),
            None => self.nodes.iter().filter(|(_, n)| n.get("type").and_then(Kv::str) == Some("CANVAS")).map(|(id, _)| id.clone()).collect(),
        };
        let mut out = Vec::new();
        for p in pages {
            let Some(&n) = self.nodes.get(&p) else { continue };
            if n.get("type").and_then(Kv::str) != Some("CANVAS") {
                self.warn("document children other than pages were skipped");
                continue;
            }
            // The hidden page that holds styles, variables and library copies.
            if n.get("internalOnly").and_then(Kv::bool) == Some(true) {
                continue;
            }
            out.push(self.page(&p, n));
        }
        if out.is_empty() {
            self.warn("the file has no pages");
        }
        json!({"id": doc_id.unwrap_or_else(|| "0:0".into()), "type": "DOCUMENT", "children": out})
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
        if depth > MAX_DEPTH {
            self.warn(format!("layers nested deeper than {MAX_DEPTH} levels were skipped"));
            return None;
        }
        if self.count >= MAX_NODES {
            self.warn(format!("the file has more than {MAX_NODES} layers; the rest were skipped"));
            return None;
        }
        let base = *self.nodes.get(id)?;
        let path = inst.map(|i| [i.path.as_slice(), &[id.to_string()]].concat());
        let entries = match (inst, &path) {
            (Some(i), Some(p)) => i.overrides.get(p).map(Vec::as_slice).unwrap_or_default(),
            _ => &[],
        };
        let mut view = View::of(base).with(entries.iter().map(|e| e.1));
        // A layer of a resized instance the file gives no derived layout for follows its
        // constraints from the main's geometry, then takes the size and place overridden on it
        // (auto layout places the others, as in the editor).
        let derived = entries.iter().any(|(d, l)| *d == Entry::Derived && (l.get("size").is_some() || l.get("transform").is_some()));
        let stacked = parent.and_then(|p| p.str("stackMode")).is_some_and(|m| matches!(m, "HORIZONTAL" | "VERTICAL" | "GRID"))
            && view.str("stackPositioning") != Some("ABSOLUTE");
        let main = View::of(base);
        let set = |k: &str| entries.iter().rev().filter(|e| e.0 != Entry::Stale).find_map(|(_, l)| l.get(k));
        if let (Some(f), false, false, Some((w, h))) = (fit, derived, stacked, main.size()) {
            let t = main.transform().unwrap_or_default();
            let (fw, ft) = f.apply(w, h, t, view.str("horizontalConstraint"), view.str("verticalConstraint"));
            view.fitted = Some((set("size").and_then(size).unwrap_or(fw), set("transform").and_then(matrix).unwrap_or(ft)));
        } else if entries.iter().any(|(d, l)| *d == Entry::Stale && (l.get("size").is_some() || l.get("transform").is_some()))
            && let Some(s) = set("size").and_then(size).or_else(|| main.size())
        {
            view.fitted = Some((s, set("transform").and_then(matrix).or_else(|| main.transform()).unwrap_or_default()));
        }
        let rest_id = match (inst, &path) {
            (Some(i), Some(p)) => format!("I{};{}", i.root, p.join(";")),
            _ => id.to_string(),
        };
        let own = view.str("type").unwrap_or("");
        let stand = stand_in(own);
        let ty = match own {
            "ROUNDED_RECTANGLE" => "RECTANGLE",
            "SYMBOL" => "COMPONENT",
            "FRAME" if view.flag("isStateGroup") == Some(true) => "COMPONENT_SET",
            "FRAME" if view.flag("resizeToFit") == Some(true) => "GROUP",
            "VARIABLE" | "VARIABLE_SET" => return None,
            t => stand.map_or(t, |s| s.0),
        };
        if let Some((_, what)) = stand {
            self.warn(format!("{} layers import as {what}", own.to_ascii_lowercase().replace('_', " ")));
        }
        self.count += 1;
        let mut o = Obj::new();
        o.insert("id".into(), json!(rest_id));
        o.insert("type".into(), json!(ty));
        copy_str(&mut o, &view, "name", "name");
        self.common(&mut o, &view, ty, space, parent);
        match ty {
            "TEXT" => self.text(&mut o, &view),
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
        if own == "CONNECTOR" {
            self.connector(&mut o, &view, space);
        }
        if ty == "COMPONENT" || ty == "COMPONENT_SET" {
            self.component(&mut o, &view, &rest_id);
        }
        self.prop_refs(&mut o, &view, inst);
        let child_space = if matches!(ty, "GROUP" | "BOOLEAN_OPERATION") { space * view.transform().unwrap_or_default() } else { Affine::IDENTITY };
        let mut kids = Vec::new();
        if ty == "INSTANCE" {
            if let Some((sub, sym)) = self.instance(&mut o, &view, id, inst, &rest_id) {
                let main = self.nodes.get(&sym).map(|m| View::of(m));
                let fit = Fit::frame(main.and_then(|m| m.size()), view.size());
                for c in self.children(&sym) {
                    kids.extend(self.layer(&c, Some(&sub), child_space, Some(&view), fit, depth + 1));
                }
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
                kids.extend(self.layer(&c, inst, child_space, Some(&view), fit, depth + 1));
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
        if own != "SHAPE_WITH_TEXT" || matches!(shape, "SQUARE" | "ROUNDED_RECTANGLE") || !outlined || self.count >= MAX_NODES {
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
        if self.count >= MAX_NODES {
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
        if own_text(v).is_some() {
            self.warn("connector labels were skipped");
        }
        if ["vectorNetwork", "fillGeometry", "strokeGeometry"].iter().any(|k| o.contains_key(*k)) {
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
        o.insert("fills".into(), Value::Array(fills.iter().filter_map(paint).collect()));
        let stroke_style = self.style(v, "styleIdForStrokeFill", "inheritFillStyleIDForStroke", "stroke", &mut styles);
        let strokes = own("strokePaints")
            .unwrap_or_else(|| stroke_style.map_or_else(|| v.list("strokePaints"), |s| s.get("fillPaints").map(Kv::list).unwrap_or_default()));
        o.insert("strokes".into(), Value::Array(strokes.iter().filter_map(paint).collect()));
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
        let id = v.get(key).and_then(ref_guid).or_else(|| v.get(legacy).and_then(guid).filter(|g| g != NO_STYLE))?;
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
            Some("SPACE_BETWEEN" | "SPACE_EVENLY" | "SPACE_AROUND" | "SPACE_EVENLY_CSS") => "SPACE_BETWEEN",
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
        o.insert("primaryAxisSizingMode".into(), json!(sizing("stackPrimarySizing")));
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
            let sizes = ["gridColumnsSizing", "gridRowsSizing"]
                .iter()
                .flat_map(|k| v.get(k).and_then(|m| m.get("entries")).map(Kv::list).unwrap_or_default());
            let flexible = sizes.into_iter().all(|e| {
                let t = e.get("trackSize");
                ["minSizing", "maxSizing"]
                    .iter()
                    .all(|s| t.and_then(|t| t.get(s)).and_then(|s| s.get("type")).and_then(Kv::str).is_none_or(|x| x == "FLEX"))
            });
            if !flexible {
                self.warn("grid tracks of fixed or content size import as equal tracks");
            }
        }
    }

    fn text(&mut self, o: &mut Obj, v: &View<'a>) {
        let style_node = v
            .get("styleIdForText")
            .and_then(ref_guid)
            .or_else(|| v.get("inheritTextStyleID").and_then(guid))
            .and_then(|s| self.nodes.get(&s).copied());
        let sv = v.with(style_node.filter(|s| s.get("styleType").and_then(Kv::str) == Some("TEXT")));
        let td = v.get("textData");
        let chars = td.and_then(|t| t.get("characters")).and_then(Kv::str).unwrap_or("");
        o.insert("characters".into(), json!(chars));
        let mut style = text_style(&sv, None);
        for k in ["textAlignHorizontal", "textAlignVertical", "textAutoResize", "textTruncation"] {
            copy_str(&mut style, v, k, k);
        }
        if let Some(n) = v.num("maxLines").filter(|n| *n >= 1.0) {
            style.insert("maxLines".into(), json!(n));
        }
        let size = sv.num("fontSize");
        if let Some(b) = o.get_mut("style").and_then(|s| s.get_mut("boundVariables")).map(Value::take) {
            style.insert("boundVariables".into(), b);
        }
        o.insert("style".into(), Value::Object(style));
        let Some(td) = td else { return };
        let ids: Vec<u64> = td.get("characterStyleIDs").map(Kv::list).unwrap_or_default().iter().filter_map(Kv::f64).map(|i| i as u64).collect();
        if ids.iter().any(|i| *i != 0) {
            let mut table = Obj::new();
            for e in td.get("styleOverrideTable").map(Kv::list).unwrap_or_default() {
                let Some(sid) = num(e, "styleID") else { continue };
                let mut s = text_style(&View::of(e), size);
                if let Some(fills) = e.get("fillPaints") {
                    s.insert("fills".into(), Value::Array(fills.list().iter().filter_map(paint).collect()));
                }
                table.insert((sid as u64).to_string(), Value::Object(s));
            }
            o.insert("characterStyleOverrides".into(), json!(ids));
            o.insert("styleOverrideTable".into(), Value::Object(table));
        }
        let lines: Vec<&str> = td
            .get("lines")
            .map(Kv::list)
            .unwrap_or_default()
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
        let Some(blob) = num(vd, "vectorNetworkBlob").and_then(|i| self.blobs.get(i as usize)).copied() else { return false };
        let Some(net) = network::decode(blob) else {
            self.warn("damaged vector networks import as outlines");
            return false;
        };
        // An open path shows its fill, which regions can't hold: the outline keeps the look.
        let fills_shown =
            o.get("fills").and_then(Value::as_array).is_some_and(|f| f.iter().any(|p| p.get("visible").and_then(Value::as_bool) != Some(false)));
        if net.segments.is_empty() || (net.regions.is_empty() && fills_shown && o.contains_key("fillGeometry")) {
            return false;
        }
        let styles: BTreeMap<u64, &Kv> =
            vd.get("styleOverrideTable").map(Kv::list).unwrap_or_default().iter().filter_map(|s| Some((num(s, "styleID")? as u64, s))).collect();
        let style = |id: u32| styles.get(&u64::from(id)).copied().filter(|_| id != 0);
        let scale = |k: &str| {
            let size = v.size().map(|s| if k == "x" { s.0 } else { s.1 });
            let norm = vd.get("normalizedSize").and_then(|s| num(s, k));
            size.zip(norm).filter(|(s, n)| s.is_finite() && *n > 1e-9).map_or(1.0, |(s, n)| s / n)
        };
        let (sx, sy) = (scale("x"), scale("y"));
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
                out
            })
            .collect();
        let segments: Vec<Value> =
            net.segments.iter().map(|s| json!({"a": s.a, "b": s.b, "ta": [s.ta.0 * sx, s.ta.1 * sy], "tb": [s.tb.0 * sx, s.tb.1 * sy]})).collect();
        let regions: Vec<Value> =
            net.regions.iter().map(|r| json!({"loops": r.loops, "winding": if r.non_zero { "nonZero" } else { "evenOdd" }})).collect();
        if net.regions.iter().any(|r| style(r.style).is_some_and(|s| s.get("fillPaints").is_some())) {
            self.warn("filled areas of a vector with their own fill use the layer's fill");
        }
        let join = v.str("strokeJoin");
        if net.vertices.iter().any(|p| style(p.style).and_then(|s| s.get("strokeJoin")).and_then(Kv::str).is_some_and(|j| Some(j) != join)) {
            self.warn("points with their own stroke join use the layer's");
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
        o.insert("vectorNetwork".into(), json!({"vertices": vertices, "segments": segments, "regions": regions}));
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

    /// A layer's component property references, and inside an instance the values they take.
    fn prop_refs(&mut self, o: &mut Obj, v: &View<'a>, inst: Option<&Inst<'a>>) {
        let mut refs = Obj::new();
        for e in v.get("parameterConsumptionMap").and_then(|m| m.get("entries")).map(Kv::list).unwrap_or_default() {
            let field = match e.get("variableField").and_then(Kv::str) {
                Some("VISIBLE") => "visible",
                Some("TEXT_DATA") => "characters",
                Some("OVERRIDDEN_SYMBOL_ID") => "mainComponent",
                _ => continue,
            };
            let Some(def) =
                e.get("variableData").and_then(|d| d.get("value")).and_then(|x| x.get("propRefValue")).and_then(|p| p.get("defId")).and_then(guid)
            else {
                continue;
            };
            if let Some((name, _)) = self.prop_defs.get(&def) {
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
        let mut overrides: Overrides<'a> = outer.map(|i| (*i.overrides).clone()).unwrap_or_default();
        // A nested instance's derived layout is for the size it has in its main: resized here, the
        // sizes and places in it no longer hold and its layers follow their constraints instead.
        let differs = |a: f64, b: f64| (a - b).abs() > 1e-6 * a.abs().max(b.abs()).max(1.0);
        let resized = outer.is_some()
            && match (self.nodes.get(id).and_then(|b| View::of(b).size()), v.size()) {
                (Some(a), Some(b)) => differs(a.0, b.0) || differs(a.1, b.1),
                (a, b) => a.is_some() != b.is_some(),
            };
        let lists = [sd.and_then(|s| s.get("symbolOverrides")).map(Kv::list).unwrap_or_default(), v.list("derivedSymbolData")];
        for (layer, list) in lists.into_iter().enumerate() {
            for ov in list {
                let rel: Vec<String> =
                    ov.get("guidPath").and_then(|g| g.get("guids")).map(Kv::list).unwrap_or_default().iter().filter_map(guid).collect();
                if rel.is_empty() || rel == [sym.clone()] {
                    continue;
                }
                let e = overrides.entry([own_path.as_slice(), &rel].concat()).or_default();
                // The outer instance's overrides win over this one's own; derived layout comes last.
                if layer == 0 && outer.is_some() {
                    e.insert(0, (Entry::Override, ov));
                } else {
                    e.push((
                        match (layer, resized) {
                            (0, _) => Entry::Override,
                            (_, false) => Entry::Derived,
                            (_, true) => Entry::Stale,
                        },
                        ov,
                    ));
                }
            }
        }
        let mut all_props: BTreeMap<String, &'a Kv<'a>> = outer.map(|i| (*i.props).clone()).unwrap_or_default();
        for a in assignments {
            if let (Some(d), Some(val)) = (a.get("defID").and_then(guid), a.get("value")) {
                all_props.entry(d).or_insert(val);
            }
        }
        let mut symbols = outer.map(|i| i.symbols.clone()).unwrap_or_default();
        symbols.push(sym.clone());
        let root = outer.map_or_else(|| rest_id.to_string(), |i| i.root.clone());
        Some((Inst { root, path: own_path, overrides: Rc::new(overrides), props: Rc::new(all_props), symbols }, sym))
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

fn paint(p: &Kv) -> Option<Value> {
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
    copy_str(&mut s, v, "textCase", "textCase");
    copy_str(&mut s, v, "textDecoration", "textDecoration");
    let font_size = v.num("fontSize").or(size).unwrap_or(12.0);
    if let Some(ls) = v.get("letterSpacing") {
        let val = num(ls, "value").unwrap_or(0.0);
        let px = match ls.get("units").and_then(Kv::str) {
            Some("PIXELS") => val,
            Some("PERCENT") => val / 100.0 * font_size,
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
