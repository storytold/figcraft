//! Import of the public REST API file JSON: the response of `GET /v1/files/:key` (a `document`
//! with `CANVAS` pages, plus `components`, `componentSets` and `styles` metadata) and of
//! `GET /v1/files/:key/nodes` (a `nodes` map of such entries).
//!
//! The mapping follows the published REST API reference only. Pages, the layer tree, geometry
//! (`relativeTransform`, else `absoluteBoundingBox` + `rotation`), paints, strokes, effects,
//! corners, constraints, auto layout, layout grids, export settings, text with styled runs,
//! vectors from `fillGeometry` / `strokeGeometry`, components, component sets, instances and
//! component properties are imported. Image fills keep their `imageRef` as the image key and
//! show a grey placeholder unless an [`ImageResolver`] supplies the bytes (nothing is fetched).
//! Prototyping comes along: interactions (triggers, actions, transitions, easing), flows,
//! scrolling and the prototype device. Variables come from the local variables response
//! (`GET /v1/files/:key/variables/local`), embedded next to the file JSON or passed separately
//! ([`import_with`]); layers' `boundVariables` and `explicitVariableModes` link to them.
//! Anything unknown is skipped and reported in the warning list; malformed input never panics.
//!
//! Instances come out with their own layers as the file lists them. Linking those layers to the
//! main component's (`source`) and deriving overrides is left to the caller (the engine does it
//! after loading), since that needs the component model. The private .fig entry point additionally
//! preserves explicit GUID/expansion provenance before the engine derives overrides.

#[cfg(test)]
mod blur_tests;
#[cfg(test)]
mod conditional_tests;
#[cfg(test)]
mod effects_tests;
mod fig_sources;
#[cfg(test)]
mod gamepad_tests;
#[cfg(test)]
mod import_regression_tests;
pub(crate) mod props;
pub(crate) mod proto;
#[cfg(test)]
mod proto_variable_tests;
#[cfg(test)]
mod tests;
mod text;
mod variables;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use figcraft_doc::geom::{Affine, BezPath, Rect, Shape};
use figcraft_doc::proto::Flow;
use figcraft_doc::{
    BoolOp, Document, ImageData, LayoutMode, Node, NodeId, NodeKind, PropDef, Style, StyleKind, VariableType, VectorNetwork, WindingRule,
};
use serde_json::Value;

use crate::FormatError;
use props::{MAX_COORD, MAX_SIZE, clip_str, flag, list, num, text};
pub use variables::export as export_variables;

/// Most layers imported from one file. The `.fig` import sets its own, from the file's size.
pub const MAX_NODES: usize = 200_000;
/// Deepest layer nesting followed.
const MAX_DEPTH: usize = 256;
/// Private .fig provenance, consumed only by the `.fig` import ([`import_with_networks`]).
pub(crate) const FIG_SOURCE_FIELD: &str = "_figSource";
/// Hidden .fig library context; accepted only by the `.fig` import ([`import_with_networks`]).
pub(crate) const FIG_LIBRARY_CONTEXTS_FIELD: &str = "_figLibraryContexts";
/// Longest SVG path string read from one geometry entry.
const MAX_PATH_BYTES: usize = 4 << 20;
/// Most path elements in one vector.
const MAX_PATH_ELEMENTS: usize = 200_000;
/// Most distinct warnings reported.
pub(crate) const MAX_WARNINGS: usize = 200;
/// Most shared style definitions read.
const MAX_STYLES: usize = 10_000;
/// Image key used for image fills without an `imageRef`.
const MISSING_IMAGE: &str = "rest-image-missing";

/// Image bytes for an `imageRef` (the REST API serves image fills separately).
pub type ImageResolver<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;

/// An [`ImageResolver`] for when no image files are at hand.
pub fn no_images(_: &str) -> Option<Vec<u8>> {
    None
}

/// Whether `v` is a REST API file response or a nodes response.
pub fn is_rest_json(v: &Value) -> bool {
    is_document(v) || nodes_entries(v).next().is_some()
}

fn is_document(entry: &Value) -> bool {
    entry.get("document").and_then(|d| text(d, "type")) == Some("DOCUMENT")
}

/// Entries of a nodes response (`{"nodes": {"1:2": {"document": …, "components": …}}}`).
fn nodes_entries(v: &Value) -> impl Iterator<Item = &Value> {
    v.get("nodes")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|m| m.values())
        .filter(|e| e.get("document").and_then(|d| text(d, "type")).is_some())
}

/// Whether `v` is a local variables response (or its `meta`).
pub fn is_variables_json(v: &Value) -> bool {
    variables::source(v).is_some()
}

/// The collections and variables of a local variables response (or its `meta`), checked like a
/// file's, with the warnings about what was skipped; None when `v` isn't one.
pub fn read_variables(v: &Value) -> Option<(figcraft_doc::Variables, Vec<String>)> {
    let src = variables::source(v)?;
    let mut cx = Ctx::new(Document::new("Variables"), &no_images);
    cx.variables(src);
    Some((std::mem::take(&mut cx.doc.variables), cx.warnings.finish()))
}

/// Import `bytes` when they are REST API file JSON; `None` when they are something else (the
/// caller then tries its other formats).
pub fn try_import(bytes: &[u8], images: ImageResolver) -> Option<Result<(Document, Vec<String>), FormatError>> {
    try_import_with(bytes, None, images)
}

/// [`try_import`] with the local variables response (or its `meta`) for the file, when the file
/// JSON doesn't embed it.
pub fn try_import_with(bytes: &[u8], variables: Option<&Value>, images: ImageResolver) -> Option<Result<(Document, Vec<String>), FormatError>> {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace())?;
    if bytes.get(start) != Some(&b'{') {
        return None;
    }
    let src = std::str::from_utf8(bytes).ok()?;
    if !src.contains("\"document\"") {
        return None;
    }
    match serde_json::from_str::<Value>(src) {
        Ok(v) => is_rest_json(&v).then(|| import_with(&v, variables, images)),
        // Broken JSON that names a DOCUMENT root: report it as such.
        Err(e) => src.contains("\"DOCUMENT\"").then(|| Err(FormatError::Corrupt(format!("REST API file JSON: {e}")))),
    }
}

/// Build a document from a parsed REST API file (or nodes) response. Returns it with the
/// warnings about what was skipped or approximated.
pub fn import(v: &Value, images: ImageResolver) -> Result<(Document, Vec<String>), FormatError> {
    import_with(v, None, images)
}

/// [`import`] with the local variables response (or its `meta`) for the file. Variables embedded
/// in the file JSON (a `meta` with `variableCollections`, or `variableCollections` and
/// `variables` at the top level) are read too; the first definition of an id wins.
pub fn import_with(v: &Value, variables: Option<&Value>, images: ImageResolver) -> Result<(Document, Vec<String>), FormatError> {
    import_parts(v, variables, images, Vec::new(), MAX_NODES, false)
}

/// Internal .fig JSON path: GUID provenance is not part of the public REST import contract.
/// The `.fig` import itself goes through [`import_with_networks`].
#[cfg(test)]
pub(crate) fn import_fig(v: &Value, images: ImageResolver) -> Result<(Document, Vec<String>), FormatError> {
    import_with_networks(v, images, Vec::new(), MAX_NODES)
}

/// [`import_fig`] for the `.fig` import's JSON, whose vectors name their networks by
/// `vectorNetworkRef`, an index in `networks` (built once each, and far smaller than as JSON).
/// `max_nodes`: the layer cap, set by the `.fig` import in place of [`MAX_NODES`].
pub(crate) fn import_with_networks(
    v: &Value,
    images: ImageResolver,
    networks: Vec<Option<VectorNetwork>>,
    max_nodes: usize,
) -> Result<(Document, Vec<String>), FormatError> {
    import_parts(v, None, images, networks, max_nodes, true)
}

fn import_parts(
    v: &Value,
    variables: Option<&Value>,
    images: ImageResolver,
    networks: Vec<Option<VectorNetwork>>,
    max_nodes: usize,
    fig_sources: bool,
) -> Result<(Document, Vec<String>), FormatError> {
    let entries: Vec<&Value> = if is_document(v) { vec![v] } else { nodes_entries(v).collect() };
    if entries.is_empty() {
        return Err(FormatError::Corrupt("not a REST API file JSON (no `document`)".into()));
    }
    let name = text(v, "name").filter(|n| !n.trim().is_empty()).map_or("Imported file", clip_str);
    let mut cx = Ctx::new(Document::new(name), images);
    cx.fig_sources = fig_sources.then(|| fig_sources::Sources::new(max_nodes));
    let blank = cx.doc.first_page();
    let mut loose: Option<NodeId> = None;
    for e in &entries {
        cx.collect_meta(e);
    }
    // Variables first: layers link to them as they are built.
    for src in [variables::source(v), variables.and_then(variables::source)].into_iter().flatten() {
        cx.variables(src);
    }
    if variables.is_some_and(|x| variables::source(x).is_none()) {
        cx.warn("the variables JSON has no `variableCollections` (expected the local variables response) and was ignored");
    }
    cx.style_definitions(v);
    cx.networks = networks;
    cx.max_nodes = max_nodes;
    for e in entries {
        let Some(root) = e.get("document") else { continue };
        match text(root, "type") {
            Some("DOCUMENT") => {
                for c in children(root) {
                    if text(c, "type") == Some("CANVAS") {
                        cx.page(c);
                    } else {
                        cx.warn("document children other than pages were skipped");
                    }
                }
            }
            Some("CANVAS") => cx.page(root),
            // A nodes response for layers: they share one page, placed where they are.
            Some(_) => {
                let page = *loose.get_or_insert_with(|| cx.doc.add_page("Imported layers"));
                cx.build(root, page, Space::default(), None, 0, true);
            }
            None => cx.warn("entries without a node type were skipped"),
        }
    }
    if fig_sources {
        for context in v.get(FIG_LIBRARY_CONTEXTS_FIELD).and_then(Value::as_array).into_iter().flatten() {
            let store = cx.doc.ensure_library_store();
            cx.build(context, store, Space::default(), None, 0, false);
        }
    }
    // Drop the empty page the new document started with once real pages exist.
    if let Some(p) = blank
        && cx.doc.pages.len() > 1
    {
        let _ = cx.doc.remove(p);
    }
    cx.doc.current_page = cx.doc.first_page();
    cx.styles_take_bindings();
    cx.finish();
    let warnings = cx.warnings.finish();
    Ok((cx.doc, warnings))
}

/// The key a style keeps a layer's binding `key` under, when the layer's `slot` link (fill,
/// stroke, effect, grid, text) to the style covers that field: a paint style's paints are its
/// `fills`, whichever list of the layer shows them.
fn style_binding(slot: &str, key: &str) -> Option<String> {
    let paints = |rest: &str| format!("fills/{rest}");
    match (slot, key) {
        ("fill", "fill" | "fills" | "fills/0") | ("stroke", "stroke") => Some("fills/0/color".into()),
        ("fill", k) => k.strip_prefix("fills/").map(paints),
        ("stroke", k) => k.strip_prefix("strokes/").map(paints),
        ("effect", k) if k.starts_with("effects/") => Some(k.into()),
        ("grid", k) if k.starts_with("layoutGrids/") => Some(k.into()),
        ("text", "fontSize" | "lineHeight" | "letterSpacing" | "fontFamily" | "fontStyle") => Some(key.into()),
        _ => None,
    }
}

/// Put back the shape a layer was before the `.fig` export wrote it as the outline of its stroke
/// (`outlinedStroke`: `{kind, fills, strokes, stroke}` as the document holds them). Anything that
/// isn't such a shape is ignored.
fn restore_stroke(n: &mut Node, original: &Value) -> bool {
    #[derive(serde::Deserialize)]
    struct Original {
        kind: NodeKind,
        fills: Vec<figcraft_doc::color::Paint>,
        strokes: Vec<figcraft_doc::color::Paint>,
        stroke: figcraft_doc::StrokeProps,
    }
    let Ok(mut o) = serde_json::from_value::<Original>(original.clone()) else { return false };
    match &mut o.kind {
        NodeKind::Vector { network } => {
            if network.vertices.len() + network.segments.len() > MAX_PATH_ELEMENTS {
                return false;
            }
            network.sanitize();
        }
        NodeKind::Line | NodeKind::Ellipse { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. } | NodeKind::Rectangle => {}
        _ => return false,
    }
    let sp = &mut o.stroke;
    sp.weight = sp.weight.clamp(0.0, props::MAX_SIZE);
    sp.width_profile = sp.width_profile.take().map(|p| p.sanitized());
    sp.brush = sp.brush.take().map(|b| b.sanitized());
    sp.dashes.truncate(64);
    o.fills.truncate(64);
    o.strokes.truncate(64);
    (n.kind, n.fills, n.strokes, n.stroke) = (o.kind, o.fills, o.strokes, o.stroke);
    true
}

/// A stroke-only vector imported as the outline of its stroke: the stroke paints fill the
/// outline, and the stroke's shared style and colour variables move with them. The hidden fills
/// it replaces go, with their style and bindings.
fn outline_stroke(n: &mut Node) {
    n.fills = std::mem::take(&mut n.strokes);
    n.styles.fill = n.styles.stroke.take();
    let bound = std::mem::take(&mut n.bound);
    for (key, var) in bound {
        let moved = match key.as_str() {
            "fill" => continue,
            k if k.starts_with("fills/") => continue,
            "stroke" => "fill".to_string(),
            k => match k.strip_prefix("strokes/") {
                Some(rest) => format!("fills/{rest}"),
                None => key,
            },
        };
        n.bound.insert(moved, var);
    }
}

fn children(v: &Value) -> &[Value] {
    v.get("children").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

/// Distinct warnings in first-seen order, with repeat counts.
#[derive(Default)]
struct Warnings {
    list: Vec<(String, usize)>,
    index: BTreeMap<String, usize>,
}

impl Warnings {
    fn add(&mut self, msg: String) {
        if let Some(i) = self.index.get(&msg) {
            if let Some(e) = self.list.get_mut(*i) {
                e.1 += 1;
            }
        } else if self.list.len() < MAX_WARNINGS {
            self.index.insert(msg.clone(), self.list.len());
            self.list.push((msg, 1));
        }
    }

    fn finish(self) -> Vec<String> {
        self.list.into_iter().map(|(m, n)| if n > 1 { format!("{m} (×{n})") } else { m }).collect()
    }
}

/// The coordinate spaces a child is placed in: its parent's (`world`) and that of the nearest
/// ancestor that is not a group or boolean (`frame`), both relative to the page.
#[derive(Clone, Copy)]
struct Space {
    world: Affine,
    frame: Affine,
}

impl Default for Space {
    fn default() -> Self {
        Self { world: Affine::IDENTITY, frame: Affine::IDENTITY }
    }
}

pub(crate) struct Ctx<'a> {
    doc: Document,
    images: ImageResolver<'a>,
    /// `imageRef` → key in `doc.images`.
    image_keys: BTreeMap<String, String>,
    missing_images: usize,
    warnings: Warnings,
    count: usize,
    /// REST node id → imported node.
    ids: BTreeMap<String, NodeId>,
    fig_sources: Option<fig_sources::Sources<'a>>,
    component_meta: BTreeMap<&'a str, &'a Value>,
    set_meta: BTreeMap<&'a str, &'a Value>,
    style_meta: BTreeMap<&'a str, &'a Value>,
    /// Instances (imported as frames until their main is resolved) with their JSON.
    instances: Vec<(NodeId, &'a Value)>,
    /// Instance-swap property defaults: (component or set, property, REST id of the default).
    swap_defaults: Vec<(NodeId, String, String)>,
    /// Property definitions of component sets, shared by their variants.
    set_defs: BTreeMap<NodeId, BTreeMap<String, PropDef>>,
    /// Prototype starting points: (page, name, REST node id).
    flows: Vec<(NodeId, String, String)>,
    /// Layers with prototype interactions, resolved once every layer is imported.
    protos: Vec<(NodeId, &'a Value)>,
    /// Layers that carry overlay settings (position, background, outside clicks), by REST id.
    overlay_meta: BTreeMap<&'a str, &'a Value>,
    /// Imported variables by id, and the mode ids of imported collections (default first).
    var_kinds: BTreeMap<String, VariableType>,
    var_modes: BTreeMap<String, Vec<String>>,
    /// Vector networks built by the `.fig` import, named by layers' `vectorNetworkRef`.
    networks: Vec<Option<VectorNetwork>>,
    /// Most layers imported: [`MAX_NODES`], or the `.fig` import's own cap.
    max_nodes: usize,
}

impl<'a> Ctx<'a> {
    fn new(doc: Document, images: ImageResolver<'a>) -> Self {
        Self {
            doc,
            images,
            image_keys: BTreeMap::new(),
            missing_images: 0,
            warnings: Warnings::default(),
            count: 0,
            ids: BTreeMap::new(),
            fig_sources: None,
            component_meta: BTreeMap::new(),
            set_meta: BTreeMap::new(),
            style_meta: BTreeMap::new(),
            instances: Vec::new(),
            swap_defaults: Vec::new(),
            set_defs: BTreeMap::new(),
            flows: Vec::new(),
            protos: Vec::new(),
            overlay_meta: BTreeMap::new(),
            var_kinds: BTreeMap::new(),
            var_modes: BTreeMap::new(),
            networks: Vec::new(),
            max_nodes: MAX_NODES,
        }
    }

    pub(crate) fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.add(msg.into());
    }

    fn collect_meta(&mut self, e: &'a Value) {
        for (key, into) in [("components", &mut self.component_meta), ("componentSets", &mut self.set_meta), ("styles", &mut self.style_meta)] {
            for (k, v) in e.get(key).and_then(Value::as_object).into_iter().flatten() {
                into.entry(k.as_str()).or_insert(v);
            }
        }
    }

    /// A CANVAS → a page.
    fn page(&mut self, c: &'a Value) {
        let page = self.doc.add_page(text(c, "name").map_or("Page", clip_str));
        let modes = self.modes(c);
        if let Some(n) = self.doc.node_mut(page).filter(|_| !modes.is_empty()) {
            n.var_modes = modes;
        }
        if let Some(bg) = c.get("backgroundColor").and_then(props::color)
            && let Some(NodeKind::Page { background, .. }) = self.doc.node_mut(page).map(|n| &mut n.kind)
        {
            *background = bg;
        }
        if let Some(s) = proto::page_settings(c)
            && let Some(NodeKind::Page { prototype, .. }) = self.doc.node_mut(page).map(|n| &mut n.kind)
        {
            *prototype = s;
        }
        for f in list(c, "flowStartingPoints") {
            if let Some(start) = text(f, "nodeId") {
                self.flows.push((page, clip_str(text(f, "name").unwrap_or("Flow")).to_string(), start.to_string()));
            }
        }
        // The deprecated single starting frame, for files without flows.
        if list(c, "flowStartingPoints").is_empty()
            && let Some(start) = text(c, "prototypeStartNodeID").filter(|s| !s.is_empty())
        {
            self.flows.push((page, "Flow 1".into(), start.to_string()));
        }
        for ch in children(c) {
            self.build(ch, page, Space::default(), None, 0, false);
        }
    }

    /// Import layer `j` (and its subtree) under `parent`. `root_abs`: place it by its absolute
    /// bounds (roots of a nodes response, whose parent isn't in the file).
    fn build(&mut self, j: &'a Value, parent: NodeId, space: Space, parent_mode: Option<LayoutMode>, depth: usize, root_abs: bool) {
        if depth > MAX_DEPTH {
            self.warn(format!("layers nested deeper than {MAX_DEPTH} levels were skipped"));
            return;
        }
        if self.count >= self.max_nodes {
            let max = self.max_nodes;
            self.warn(format!("the file has more than {max} layers; the rest were skipped"));
            return;
        }
        let Some(ty) = text(j, "type") else {
            self.warn("layers without a type were skipped");
            return;
        };
        let Some(kind) = self.kind(ty, j) else {
            self.warn(format!("{} layers are not supported and were skipped", clip_str(ty)));
            return;
        };
        let (world, w, h) = self.placement(j, space, root_abs);
        let id = self.doc.alloc_id();
        let label = kind.label();
        let mut n = Node::new(id, kind, text(j, "name").map_or(label, clip_str), w, h);
        // Overrides the .fig names explicitly (the rest are found by comparing with the main).
        for (flag, group) in [("slotOverride", "slot"), ("sizeOverride", "size")] {
            if self.fig_sources.is_some() && j.get(FIG_SOURCE_FIELD).and_then(|source| source.get(flag)).and_then(Value::as_bool) == Some(true) {
                n.overrides.insert(group.into());
            }
        }
        n.transform = local(space.world, world);
        self.common(&mut n, j, parent_mode);
        let (container, follows, mode) = (n.kind.is_container(), n.kind.bounds_follow_children(), n.layout.as_ref().map(|l| l.mode));
        if self.doc.insert(n, parent, None).is_err() {
            return;
        }
        self.count += 1;
        if let Some(rid) = text(j, "id") {
            let primary = *self.ids.entry(rid.to_string()).or_insert(id) == id;
            if let Some(sources) = &mut self.fig_sources {
                sources.record(id, j, primary);
            }
        }
        if ty == "INSTANCE" {
            self.instances.push((id, j));
        }
        let kids = children(j);
        if kids.is_empty() {
            return;
        }
        if !container {
            self.warn(format!("children of {} layers were skipped", clip_str(ty)));
            return;
        }
        let inner = Space { world, frame: if follows { space.frame } else { world } };
        for c in kids {
            self.build(c, id, inner, mode, depth + 1, false);
        }
        if follows {
            let _ = self.doc.fit_to_children(id);
        }
    }

    fn kind(&mut self, ty: &str, j: &'a Value) -> Option<NodeKind> {
        let point_count = || num(j, "pointCount").filter(|c| *c >= 3.0).map(|c| c.min(1000.0) as u32);
        let has_geometry = !list(j, "fillGeometry").is_empty() || !list(j, "strokeGeometry").is_empty();
        Some(match ty {
            // Instances start as frames; `finish` links them to their main.
            "FRAME" | "INSTANCE" => NodeKind::Frame,
            "GROUP" => NodeKind::Group,
            "SECTION" => NodeKind::Section { contents_hidden: flag(j, "sectionContentsHidden").unwrap_or(false) },
            "RECTANGLE" => NodeKind::Rectangle,
            "LINE" => NodeKind::Line,
            "ELLIPSE" => {
                let a = j.get("arcData");
                let arc = |k: &str| a.and_then(|a| num(a, k));
                let start = arc("startingAngle").unwrap_or(0.0).clamp(-1e3, 1e3);
                let end = arc("endingAngle").map_or(start + std::f64::consts::TAU, |e| e.clamp(-1e3, 1e3));
                let tau = std::f64::consts::TAU;
                NodeKind::Ellipse { start, sweep: (end - start).clamp(-tau, tau), inner: arc("innerRadius").unwrap_or(0.0).clamp(0.0, 1.0) }
            }
            // Point counts aren't part of the documented JSON: the outline is exact when present.
            "REGULAR_POLYGON" | "STAR" if point_count().is_none() && has_geometry => NodeKind::Vector { network: VectorNetwork::default() },
            "REGULAR_POLYGON" => NodeKind::Polygon { count: point_count().unwrap_or_else(|| self.default_count(3)) },
            "STAR" => NodeKind::Star {
                count: point_count().unwrap_or_else(|| self.default_count(5)),
                inner_ratio: num(j, "innerRadius").map_or(0.382, |r| r.clamp(0.01, 1.0)),
            },
            "VECTOR" => NodeKind::Vector { network: VectorNetwork::default() },
            "BOOLEAN_OPERATION" => NodeKind::BooleanOp {
                op: match text(j, "booleanOperation") {
                    Some("SUBTRACT") => BoolOp::Subtract,
                    Some("INTERSECT") => BoolOp::Intersect,
                    Some("EXCLUDE") => BoolOp::Exclude,
                    _ => BoolOp::Union,
                },
            },
            "TEXT" => NodeKind::Text(text::text_data(self, j)),
            "COMPONENT" => NodeKind::Component { description: description(&self.component_meta, j), props: BTreeMap::new() },
            "COMPONENT_SET" => NodeKind::ComponentSet { description: description(&self.set_meta, j) },
            "SLICE" => NodeKind::Slice,
            _ => return None,
        })
    }

    fn default_count(&mut self, count: u32) -> u32 {
        self.warn("stars and polygons without point counts or geometry use the default point count");
        count
    }

    /// The layer's transform relative to the page and its size.
    fn placement(&mut self, j: &Value, space: Space, root_abs: bool) -> (Affine, f64, f64) {
        let bbox = j.get("absoluteBoundingBox").and_then(rect);
        let rot = num(j, "rotation").unwrap_or(0.0).clamp(-1e3, 1e3);
        let (w, h) = j
            .get("size")
            .and_then(props::vec2)
            .map(|(x, y)| (x.clamp(0.0, MAX_SIZE), y.clamp(0.0, MAX_SIZE)))
            .or_else(|| bbox.map(|b| unrotated_size(b, rot)))
            .unwrap_or((0.0, 0.0));
        let box_rect = Rect::new(0.0, 0.0, w, h);
        // Rotation is counter-clockwise on screen (y down), about the box centre.
        let from_bbox = |b: Rect| Affine::translate(b.center().to_vec2()) * Affine::rotate(-rot) * Affine::translate((-w / 2.0, -h / 2.0));
        let world = match (j.get("relativeTransform").and_then(props::transform), bbox) {
            (Some(_), Some(b)) if root_abs => from_bbox(b),
            (Some(rel), Some(b)) => {
                // Children of groups and booleans are placed in the enclosing frame's space; the
                // absolute bounds settle which space the file used.
                let err = |xf: Affine| {
                    let r = figcraft_doc::geom::transform_rect_bbox(xf, box_rect);
                    (r.x0 - b.x0).abs() + (r.y0 - b.y0).abs() + (r.x1 - b.x1).abs() + (r.y1 - b.y1).abs()
                };
                let (a, c) = (space.frame * rel, space.world * rel);
                if err(c) < err(a) { c } else { a }
            }
            (Some(rel), None) => space.frame * rel,
            (None, Some(b)) => from_bbox(b),
            (None, None) => {
                self.warn("layers without a position were placed at their parent's origin");
                space.world
            }
        };
        (world, w, h)
    }

    /// Properties shared by every layer type.
    fn common(&mut self, n: &mut Node, j: &'a Value, parent_mode: Option<LayoutMode>) {
        n.visible = flag(j, "visible").unwrap_or(true);
        n.locked = flag(j, "locked").unwrap_or(false);
        n.opacity = num(j, "opacity").map_or(1.0, |o| o.clamp(0.0, 1.0) as f32);
        if let Some(b) = text(j, "blendMode").and_then(props::blend_mode) {
            n.blend = b;
        }
        n.is_mask = flag(j, "isMask").unwrap_or(false);
        n.mask_type = match text(j, "maskType") {
            Some("VECTOR") => figcraft_doc::MaskType::Vector,
            Some("LUMINANCE") => figcraft_doc::MaskType::Luminance,
            _ => figcraft_doc::MaskType::Alpha,
        };
        // The JSON lists every paint; a missing list means none (text keeps its default black).
        match props::paints(self, j, "fills") {
            Some(f) if !matches!(n.kind, NodeKind::Group) => n.fills = f,
            None if !matches!(n.kind, NodeKind::Text(_)) => n.fills.clear(),
            _ => {}
        }
        n.strokes = props::paints(self, j, "strokes").unwrap_or_default();
        props::stroke_props(j, &mut n.stroke);
        n.effects = props::effects(self, j);
        if n.kind.has_corner_radius() {
            props::corners(j, &mut n.corners.radii, &mut n.corners.smoothing);
        }
        if let Some(c) = props::constraints(j) {
            n.constraints = c;
        }
        if n.kind.is_frame_like() {
            if let Some(c) = flag(j, "clipsContent") {
                n.clip = c;
            }
            n.layout = props::auto_layout(j, self.fig_sources.is_some());
            n.grids = props::layout_grids(j);
        }
        n.child_layout = props::child_layout(j, n.layout.as_ref().map(|l| l.mode), parent_mode);
        n.exports = props::export_settings(j);
        for (field, prop) in j.get("componentPropertyReferences").and_then(Value::as_object).into_iter().flatten() {
            let supported = matches!(field.as_str(), "visible" | "characters" | "mainComponent") || (field == "slot" && self.fig_sources.is_some());
            if let (Some(p), true) = (prop.as_str(), supported) {
                n.prop_refs.insert(field.clone(), p.to_string());
            }
        }
        let outlined = matches!(n.kind, NodeKind::Vector { .. }) && self.vector(n, j);
        if matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. }) {
            let defs = self.prop_defs(n.id, j);
            match &mut n.kind {
                NodeKind::Component { props, .. } => *props = defs,
                _ => {
                    self.set_defs.insert(n.id, defs);
                }
            }
        }
        self.link_styles(n, j);
        proto::scrolling(n, j);
        if !list(j, "interactions").is_empty() || j.get("transitionNodeID").is_some_and(Value::is_string) {
            self.protos.push((n.id, j));
        }
        if let Some(rid) = text(j, "id")
            && ["overlayPositionType", "overlayBackground", "overlayBackgroundInteraction"].iter().any(|k| j.get(k).is_some())
        {
            self.overlay_meta.entry(rid).or_insert(j);
        }
        self.bindings(n, j);
        let restored = j.get("outlinedStroke").is_some_and(|original| restore_stroke(n, original));
        // Styles and bindings were read against the stroke: they follow its paints to the fill.
        if outlined && !restored {
            outline_stroke(n);
        }
    }

    /// A vector's network: the editable one when the JSON carries it (`vectorNetwork`, or
    /// `vectorNetworkRef` naming one built by the `.fig` import: our extensions), else from its
    /// geometry. A stroke-only vector uses the stroke outline, painted with the stroke paints
    /// (exact look, editable as a shape): returns whether it does, and the caller moves the paints
    /// with [`outline_stroke`] once the styles and variables of the stroke are read.
    fn vector(&mut self, n: &mut Node, j: &Value) -> bool {
        let net = match j.get("vectorNetworkRef") {
            Some(r) => Some(r.as_u64().and_then(|i| self.networks.get(usize::try_from(i).ok()?).cloned().flatten())),
            None => j.get("vectorNetwork").map(network),
        };
        if let Some(net) = net {
            match net {
                Some(network) => {
                    n.kind = NodeKind::Vector { network };
                    return false;
                }
                None => self.warn("invalid or oversized vector networks import as outlines"),
            }
        }
        let fills_shown = n.fills.iter().any(|p| p.visible);
        let strokes_shown = n.strokes.iter().any(|p| p.visible) && n.stroke.weight > 0.0;
        let fill = self.geometry(j, "fillGeometry");
        let stroke = self.geometry(j, "strokeGeometry");
        let ((path, winding), outline) = match (fill, stroke) {
            (_, Some(s)) if strokes_shown && !fills_shown => (s, true),
            (Some(f), _) => (f, false),
            (None, Some(s)) => (s, true),
            (None, None) => {
                self.warn("vectors without geometry are empty (export the file JSON with geometry=paths)");
                return false;
            }
        };
        let mut network = VectorNetwork::from_path(&path);
        for r in &mut network.regions {
            r.winding = winding;
        }
        n.kind = NodeKind::Vector { network };
        outline
    }

    /// The paths of a `fillGeometry` / `strokeGeometry` list (node-local SVG path data).
    fn geometry(&mut self, j: &Value, key: &str) -> Option<(BezPath, WindingRule)> {
        let mut out = BezPath::new();
        let mut winding = None;
        for p in list(j, key) {
            let Some(d) = text(p, "path") else { continue };
            if d.len() > MAX_PATH_BYTES {
                self.warn("very long vector paths were skipped");
                continue;
            }
            match BezPath::from_svg(d) {
                Ok(bp) if sane(&bp) && out.elements().len() + bp.elements().len() <= MAX_PATH_ELEMENTS => {
                    winding.get_or_insert(if text(p, "windingRule") == Some("EVENODD") { WindingRule::EvenOdd } else { WindingRule::NonZero });
                    out.extend(bp);
                }
                _ => self.warn("invalid or oversized vector path data was skipped"),
            }
        }
        (!out.elements().is_empty()).then(|| (out, winding.unwrap_or_default()))
    }

    /// Component property definitions (variant ones are implied by the variant names).
    fn prop_defs(&mut self, owner: NodeId, j: &Value) -> BTreeMap<String, PropDef> {
        let mut out = BTreeMap::new();
        for (name, d) in j.get("componentPropertyDefinitions").and_then(Value::as_object).into_iter().flatten().take(256) {
            let default = d.get("defaultValue");
            let def = match text(d, "type") {
                Some("BOOLEAN") => PropDef::Boolean { default: default.and_then(bool_of).unwrap_or(true) },
                Some("TEXT") => PropDef::Text { default: default.and_then(Value::as_str).unwrap_or("").to_string() },
                Some("INSTANCE_SWAP") => {
                    if let Some(r) = default.and_then(Value::as_str) {
                        self.swap_defaults.push((owner, name.clone(), r.to_string()));
                    }
                    continue;
                }
                Some("VARIANT") => continue,
                Some("SLOT") if self.fig_sources.is_some() => PropDef::Slot { preferred: Vec::new() },
                _ => {
                    self.warn("unknown component property types were skipped");
                    continue;
                }
            };
            out.insert(name.clone(), def);
        }
        out
    }

    /// Shared styles with their values (`styleDefinitions`, written by the `.fig` import: a list of
    /// `{id, name, description, styleType, node}`, the node being a layer that shows the style), in
    /// the file's order. Layers then link to them; styles that no layer uses come across too.
    fn style_definitions(&mut self, v: &'a Value) {
        for d in list(v, "styleDefinitions").iter().take(MAX_STYLES) {
            let (Some(id), Some(node)) = (text(d, "id").filter(|i| !i.is_empty()).map(clip_str), d.get("node")) else { continue };
            if self.doc.styles.iter().any(|s| s.id == id) {
                continue;
            }
            let kind = match text(d, "styleType") {
                Some("FILL") => StyleKind::Paint { paints: props::paints(self, node, "fills").unwrap_or_default() },
                Some("EFFECT") => StyleKind::Effect { effects: props::effects(self, node) },
                Some("GRID") => StyleKind::Grid { grids: props::layout_grids(node) },
                Some("TEXT") => StyleKind::Text { style: text::text_data(self, node).style.as_text_style() },
                _ => continue,
            };
            let name = text(d, "name").filter(|s| !s.is_empty()).map_or_else(|| format!("Style {id}"), |s| clip_str(s).to_string());
            let description = text(d, "description").map(|s| clip_str(s).to_string()).unwrap_or_default();
            // The sample layer's variable bindings on what the style holds are the style's.
            let mut sample = Node::new(NodeId(0), NodeKind::Rectangle, "", 0.0, 0.0);
            let slot = match &kind {
                StyleKind::Paint { paints } => {
                    sample.fills.clone_from(paints);
                    "fill"
                }
                StyleKind::Effect { effects } => {
                    sample.effects.clone_from(effects);
                    "effect"
                }
                StyleKind::Grid { grids } => {
                    sample.kind = NodeKind::Frame;
                    sample.grids.clone_from(grids);
                    "grid"
                }
                StyleKind::Text { .. } => {
                    sample.kind = NodeKind::Text(text::text_data(self, node));
                    "text"
                }
            };
            self.bindings(&mut sample, node);
            let bound = sample.bound.iter().filter_map(|(k, v)| Some((style_binding(slot, k)?, v.clone()))).collect();
            self.doc.styles.push(Style { id: id.to_string(), name, description, kind, bound });
        }
    }

    /// Shared styles the layer uses: the metadata names them, the layer supplies the values.
    fn link_styles(&mut self, n: &mut Node, j: &Value) {
        for (slot, sid) in j.get("styles").and_then(Value::as_object).into_iter().flatten() {
            let Some(sid) = sid.as_str().map(clip_str) else { continue };
            let (kind, target) = match slot.as_str() {
                "fill" | "fills" => (StyleKind::Paint { paints: n.fills.clone() }, &mut n.styles.fill),
                "stroke" | "strokes" => (StyleKind::Paint { paints: n.strokes.clone() }, &mut n.styles.stroke),
                "effect" | "effects" => (StyleKind::Effect { effects: n.effects.clone() }, &mut n.styles.effect),
                "grid" | "grids" => (StyleKind::Grid { grids: n.grids.clone() }, &mut n.styles.grid),
                "text" => match &n.kind {
                    NodeKind::Text(t) => (StyleKind::Text { style: t.style.as_text_style() }, &mut n.styles.text),
                    _ => continue,
                },
                _ => continue,
            };
            *target = Some(sid.to_string());
            if self.doc.styles.iter().any(|s| s.id == sid) {
                continue;
            }
            let meta = self.style_meta.get(sid);
            let name =
                meta.and_then(|m| text(m, "name")).filter(|s| !s.is_empty()).map_or_else(|| format!("Style {sid}"), |s| clip_str(s).to_string());
            let description = meta.and_then(|m| text(m, "description")).map(|s| clip_str(s).to_string()).unwrap_or_default();
            self.doc.styles.push(Style { id: sid.to_string(), name, description, kind, bound: BTreeMap::new() });
        }
    }

    /// Variables bound to what a layer takes from a shared style belong to the style: a style
    /// without bindings of its own takes them from the first layer that has some, and the layers
    /// keep none on those fields (the style gives them their values, each in its own modes).
    fn styles_take_bindings(&mut self) {
        let ids: Vec<NodeId> = self.doc.nodes().filter(|n| !n.styles.is_empty() && !n.bound.is_empty()).map(|n| n.id).collect();
        for id in ids {
            let Some((refs, bound)) = self.doc.node(id).map(|n| (n.styles.clone(), n.bound.clone())) else { continue };
            let mut moved: Vec<String> = Vec::new();
            for (slot, sid) in [("fill", refs.fill), ("stroke", refs.stroke), ("effect", refs.effect), ("grid", refs.grid), ("text", refs.text)] {
                let Some(sid) = sid else { continue };
                let found: Vec<(String, String, String)> =
                    bound.iter().filter_map(|(k, v)| Some((k.clone(), style_binding(slot, k)?, v.clone()))).collect();
                if found.is_empty() {
                    continue;
                }
                // A style that has bindings already keeps them: only what agrees leaves the layer.
                let Some(st) = self.doc.styles.iter_mut().find(|s| s.id == sid) else { continue };
                let takes = st.bound.is_empty();
                for (key, style_key, var) in found {
                    if takes {
                        st.bound.insert(style_key, var);
                    } else if st.bound.get(&style_key) != Some(&var) {
                        continue;
                    }
                    moved.push(key);
                }
            }
            if !moved.is_empty()
                && let Ok(n) = self.doc.get_mut(id)
            {
                n.bound.retain(|k, _| !moved.contains(k));
            }
        }
    }

    /// The document image key for an `imageRef`: its bytes when the resolver has them, else a
    /// grey placeholder stored under the `imageRef` itself.
    pub(crate) fn image_key(&mut self, image_ref: &str) -> String {
        if let Some(k) = self.image_keys.get(image_ref) {
            return k.clone();
        }
        let resolved = if image_ref.is_empty() { None } else { (self.images)(image_ref) };
        let key = match resolved.and_then(|b| crate::add_image(&mut self.doc, b).ok()) {
            Some((hash, _, _)) => hash,
            None => {
                self.missing_images += 1;
                let key = if image_ref.is_empty() { MISSING_IMAGE } else { clip_str(image_ref) }.to_string();
                if !self.doc.images.contains_key(&key)
                    && let Some(png) = placeholder_png()
                {
                    self.doc.images.insert(key.clone(), ImageData { bytes: Arc::new(png), width: PLACEHOLDER_SIDE, height: PLACEHOLDER_SIDE });
                }
                key
            }
        };
        self.image_keys.insert(image_ref.to_string(), key.clone());
        key
    }

    fn is_component(&self, id: NodeId) -> bool {
        self.doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. }))
    }

    /// The component a REST id names, if it is in the file.
    fn component(&self, rest_id: &str) -> Option<NodeId> {
        self.ids.get(rest_id).copied().filter(|c| self.is_component(*c))
    }

    /// Cross-references that need every layer imported first.
    fn finish(&mut self) {
        self.resolve_swap_defaults();
        // Variants share their set's property definitions.
        for (set, defs) in std::mem::take(&mut self.set_defs) {
            for v in self.doc.node(set).map(|s| s.children.clone()).unwrap_or_default() {
                if let Some(NodeKind::Component { props, .. }) = self.doc.node_mut(v).map(|n| &mut n.kind) {
                    for (k, d) in &defs {
                        props.entry(k.clone()).or_insert_with(|| d.clone());
                    }
                }
            }
        }
        self.resolve_instances();
        if let Some(sources) = self.fig_sources.take() {
            let missing = sources.apply(&mut self.doc, &self.ids);
            if missing > 0 {
                self.warn(format!("{missing} .fig instance source link(s) could not be resolved and were left unlinked"));
            }
        }
        self.rename_props();
        for (id, j) in std::mem::take(&mut self.protos) {
            let list = self.interactions(j);
            if let Some(n) = self.doc.node_mut(id) {
                n.interactions = list;
            }
        }
        for (page, name, start) in std::mem::take(&mut self.flows) {
            let Some(&id) = self.ids.get(&start) else { continue };
            if self.doc.page_of(id) == Some(page)
                && let Some(NodeKind::Page { flows, .. }) = self.doc.node_mut(page).map(|n| &mut n.kind)
            {
                flows.push(Flow { name, start: id, description: String::new() });
            }
        }
        if self.missing_images > 0 {
            let n = self.missing_images;
            let (s, verb) = if n == 1 { ("", "shows") } else { ("s", "show") };
            self.warn(format!(
                "{n} image{s} without image data {verb} a grey placeholder named by its imageRef (put the downloaded images, named by imageRef, in an `images` folder next to the JSON)"
            ));
        }
        let unused = self.style_meta.keys().filter(|k| !self.doc.styles.iter().any(|s| s.id == **k)).count();
        if unused > 0 {
            let (s, verb) = if unused == 1 { ("", "was") } else { ("s", "were") };
            self.warn(format!("{unused} style{s} that no layer uses {verb} skipped (the file JSON carries style values only on layers)"));
        }
    }

    fn resolve_swap_defaults(&mut self) {
        for (owner, name, rest_id) in std::mem::take(&mut self.swap_defaults) {
            let Some(c) = self.component(&rest_id) else {
                self.warn("instance swap properties whose default component isn't in the file were skipped");
                continue;
            };
            let def = PropDef::InstanceSwap { default: c, preferred: Vec::new() };
            if let Some(defs) = self.set_defs.get_mut(&owner) {
                defs.insert(name, def);
            } else if let Some(NodeKind::Component { props, .. }) = self.doc.node_mut(owner).map(|n| &mut n.kind) {
                props.insert(name, def);
            }
        }
    }

    /// Turn the instance frames whose main component is in the file into instances. Instances of
    /// outside components, and instances that would nest a component inside itself (directly or
    /// through other components), stay frames.
    fn resolve_instances(&mut self) {
        let mut links: Vec<(NodeId, NodeId, &'a Value)> = Vec::new();
        for (inst, j) in std::mem::take(&mut self.instances) {
            match text(j, "componentId").and_then(|r| self.component(r)) {
                None => self.warn("instances of components that aren't in the file were imported as frames"),
                Some(c) if self.doc.is_ancestor(c, inst) => self.warn("instances inside their own main component were imported as frames"),
                Some(c) => links.push((inst, c, j)),
            }
        }
        let owner = |doc: &Document, id: NodeId| {
            doc.ancestors(id).into_iter().find(|a| doc.node(*a).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. })))
        };
        let mut uses: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();
        for (inst, c, _) in &links {
            if let Some(o) = owner(&self.doc, *inst) {
                uses.entry(o).or_default().insert(*c);
            }
        }
        let mut linked = Vec::new();
        for (inst, c, j) in links {
            if owner(&self.doc, inst).is_some_and(|o| reaches(&uses, c, o)) {
                self.warn("instances that would nest a component inside itself were imported as frames");
                continue;
            }
            let props = self.instance_props(j);
            if let Some(n) = self.doc.node_mut(inst) {
                n.kind = NodeKind::Instance { component: c, props };
                linked.push((inst, j));
            }
        }
        // Overridden fields the file lists, on the instance and its layers.
        for (inst, j) in linked {
            for o in list(j, "overrides") {
                let Some(&id) = text(o, "id").and_then(|r| self.ids.get(r)) else { continue };
                if !(id == inst || self.doc.is_ancestor(inst, id)) {
                    continue;
                }
                let groups: Vec<&'static str> = list(o, "overriddenFields").iter().filter_map(Value::as_str).filter_map(override_group).collect();
                if let Some(n) = self.doc.node_mut(id) {
                    n.overrides.extend(groups.into_iter().map(str::to_string));
                }
            }
        }
    }

    /// An instance's property values (variant values are implied by the main it shows).
    fn instance_props(&self, j: &Value) -> BTreeMap<String, Value> {
        let mut out = BTreeMap::new();
        for (name, p) in j.get("componentProperties").and_then(Value::as_object).into_iter().flatten().take(256) {
            let value = p.get("value");
            let v = match text(p, "type") {
                Some("BOOLEAN") => value.and_then(bool_of).map(Value::Bool),
                Some("TEXT") => value.and_then(Value::as_str).map(|s| Value::String(s.to_string())),
                Some("INSTANCE_SWAP") => value.and_then(Value::as_str).and_then(|r| self.component(r)).map(|c| Value::from(c.0)),
                _ => None,
            };
            if let Some(v) = v {
                out.insert(name.clone(), v);
            }
        }
        out
    }

    /// Component property names drop their `#id` suffix ("Label#12:3" → "Label") unless that
    /// would merge two different properties.
    fn rename_props(&mut self) {
        let mut by_short: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let ids: Vec<NodeId> = self.doc.ids().collect();
        for n in ids.iter().filter_map(|i| self.doc.node(*i)) {
            let keys: Vec<&String> = match &n.kind {
                NodeKind::Component { props, .. } => props.keys().collect(),
                NodeKind::Instance { props, .. } => props.keys().collect(),
                _ => Vec::new(),
            };
            for k in keys.into_iter().chain(n.prop_refs.values()) {
                by_short.entry(short_prop_name(k).to_string()).or_default().insert(k.clone());
            }
        }
        let rename: BTreeMap<String, String> = by_short
            .into_iter()
            .filter(|(_, raws)| raws.len() == 1)
            .filter_map(|(short, raws)| raws.into_iter().next().filter(|r| *r != short).map(|r| (r, short)))
            .collect();
        if rename.is_empty() {
            return;
        }
        let re = |k: &String| rename.get(k).cloned().unwrap_or_else(|| k.clone());
        for id in ids {
            let Some(n) = self.doc.node(id) else { continue };
            let touches = n.prop_refs.values().any(|v| rename.contains_key(v))
                || match &n.kind {
                    NodeKind::Component { props, .. } => props.keys().any(|k| rename.contains_key(k)),
                    NodeKind::Instance { props, .. } => props.keys().any(|k| rename.contains_key(k)),
                    _ => false,
                };
            if !touches {
                continue;
            }
            let Some(n) = self.doc.node_mut(id) else { continue };
            for v in n.prop_refs.values_mut() {
                *v = re(v);
            }
            match &mut n.kind {
                NodeKind::Component { props, .. } => *props = std::mem::take(props).into_iter().map(|(k, d)| (re(&k), d)).collect(),
                NodeKind::Instance { props, .. } => *props = std::mem::take(props).into_iter().map(|(k, d)| (re(&k), d)).collect(),
                _ => {}
            }
        }
    }
}

/// "Label#12:3" → "Label".
fn short_prop_name(raw: &str) -> &str {
    match raw.rsplit_once('#') {
        Some((name, id)) if !name.is_empty() && !id.is_empty() && id.chars().all(|c| c.is_ascii_digit() || c == ':') => name,
        _ => raw,
    }
}

/// Whether component `to` is reachable from component `from` through the instances inside them.
fn reaches(uses: &BTreeMap<NodeId, BTreeSet<NodeId>>, from: NodeId, to: NodeId) -> bool {
    let mut seen = BTreeSet::new();
    let mut stack = vec![from];
    while let Some(c) = stack.pop() {
        if c == to {
            return true;
        }
        if seen.insert(c) {
            stack.extend(uses.get(&c).into_iter().flatten().copied());
        }
    }
    false
}

/// The override group of an overridden field name.
fn override_group(field: &str) -> Option<&'static str> {
    Some(match field {
        "fills" | "fillStyleId" => "fills",
        "strokes"
        | "strokeStyleId"
        | "strokeWeight"
        | "strokeAlign"
        | "strokeCap"
        | "strokeJoin"
        | "strokeDashes"
        | "individualStrokeWeights"
        | "strokeTopWeight"
        | "strokeRightWeight"
        | "strokeBottomWeight"
        | "strokeLeftWeight" => "strokes",
        "effects" | "effectStyleId" => "effects",
        "opacity" => "opacity",
        "visible" => "visible",
        "name" => "name",
        "cornerRadius"
        | "rectangleCornerRadii"
        | "topLeftRadius"
        | "topRightRadius"
        | "bottomLeftRadius"
        | "bottomRightRadius"
        | "cornerSmoothing" => "corners",
        "characters" => "characters",
        "fontName"
        | "fontFamily"
        | "fontStyle"
        | "fontWeight"
        | "fontSize"
        | "letterSpacing"
        | "lineHeight"
        | "textCase"
        | "textDecoration"
        | "textStyleId"
        | "textAlignHorizontal"
        | "textAlignVertical"
        | "textAutoResize"
        | "paragraphSpacing"
        | "paragraphIndent"
        | "listSpacing"
        | "hangingPunctuation"
        | "hangingList"
        | "textDecorationStyle"
        | "textDecorationThickness"
        | "textDecorationOffset"
        | "textDecorationSkipInk"
        | "textDecorationColor" => "textStyle",
        "width" | "height" | "size" => "size",
        "layoutSizingHorizontal"
        | "layoutSizingVertical"
        | "layoutGrow"
        | "layoutAlign"
        | "primaryAxisSizingMode"
        | "counterAxisSizingMode"
        | "minWidth"
        | "maxWidth"
        | "minHeight"
        | "maxHeight" => "resizing",
        "paddingLeft" | "paddingRight" | "paddingTop" | "paddingBottom" | "horizontalPadding" | "verticalPadding" => "padding",
        "itemSpacing" | "counterAxisSpacing" => "gap",
        "primaryAxisAlignItems" | "counterAxisAlignItems" => "alignment",
        "layoutGrids" | "gridStyleId" => "grids",
        "exportSettings" => "exports",
        "interactions" | "reactions" => "interactions",
        _ => return None,
    })
}

fn bool_of(v: &Value) -> Option<bool> {
    v.as_bool().or_else(|| match v.as_str() {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    })
}

fn description(meta: &BTreeMap<&str, &Value>, j: &Value) -> String {
    text(j, "id").and_then(|id| meta.get(id)).and_then(|m| text(m, "description")).map(|d| clip_str(d).to_string()).unwrap_or_default()
}

/// `{x, y, width, height}`.
fn rect(v: &Value) -> Option<Rect> {
    let (x, y) = (num(v, "x")?.clamp(-MAX_COORD, MAX_COORD), num(v, "y")?.clamp(-MAX_COORD, MAX_COORD));
    let (w, h) = (num(v, "width")?.clamp(0.0, MAX_SIZE), num(v, "height")?.clamp(0.0, MAX_SIZE));
    Some(Rect::new(x, y, x + w, y + h))
}

/// The box size whose bounds, rotated by `rot` radians, are `b`.
fn unrotated_size(b: Rect, rot: f64) -> (f64, f64) {
    let (bw, bh) = (b.width(), b.height());
    let (c, s) = (rot.cos().abs(), rot.sin().abs());
    if s < 1e-9 {
        return (bw, bh);
    }
    let det = c * c - s * s;
    if det.abs() > 0.05 {
        let (w, h) = ((bw * c - bh * s) / det, (bh * c - bw * s) / det);
        if w >= 0.0 && h >= 0.0 {
            return (w, h);
        }
    }
    // Near 45° the size is ambiguous: a box with the same proportions.
    (bw / (c + s), bh / (c + s))
}

/// `world` expressed in the space whose page transform is `parent`.
fn local(parent: Affine, world: Affine) -> Affine {
    let l = if parent.determinant().abs() > 1e-12 { parent.inverse() * world } else { Affine::translate(world.translation() - parent.translation()) };
    if l.as_coeffs().iter().all(|c| c.is_finite()) { l } else { Affine::IDENTITY }
}

/// Finite path data within the coordinate cap.
/// A vector network in our own JSON shape, when it's within bounds and refers only to what it has.
pub(crate) fn network(v: &Value) -> Option<VectorNetwork> {
    use serde::Deserialize;
    let mut net = VectorNetwork::deserialize(v).ok()?;
    let entries: usize = net.regions.iter().flat_map(|r| &r.loops).map(Vec::len).sum();
    let coord = |x: f64| x.is_finite() && x.abs() <= MAX_COORD;
    let fits = net.vertices.len() + net.segments.len() + entries <= MAX_PATH_ELEMENTS
        && net.vertices.iter().all(|p| coord(p.x) && coord(p.y) && coord(p.radius))
        && net.segments.iter().all(|s| [s.ta.0, s.ta.1, s.tb.0, s.tb.1].into_iter().all(coord));
    if !fits {
        return None;
    }
    for p in &mut net.vertices {
        p.radius = p.radius.clamp(0.0, MAX_SIZE);
    }
    net.sanitize();
    (!net.segments.is_empty()).then_some(net)
}

fn sane(p: &BezPath) -> bool {
    let b = p.bounding_box();
    [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD)
}

/// Side of the placeholder image (large enough that scaling it up stays flat).
const PLACEHOLDER_SIDE: u32 = 64;

/// A light-grey PNG standing in for image fills whose bytes aren't available.
fn placeholder_png() -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_pixel(PLACEHOLDER_SIDE, PLACEHOLDER_SIDE, image::Rgba([0xD9, 0xD9, 0xD9, 0xFF]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
    Some(out)
}
