use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Write};

use figcraft_doc::color::{BlendMode, Color, Gradient, GradientKind, ImageScaleMode, Paint, PaintKind};
use figcraft_doc::geom::kurbo::PathEl;
use figcraft_doc::geom::{Affine, BezPath, Point};
use figcraft_doc::proto::{Action, Direction, Easing, Interaction, OverlayPosition, Transition, Trigger};
use figcraft_doc::{
    BoolOp, CellAlign, ContentAlign, CrossAlign, Document, EffectKind, GridAlign, GridKind, GridTrack, HConstraint, LayoutMode, LetterSpacing,
    LineHeight, MainAlign, Node, NodeId, NodeKind, Sizing, StrokeAlign, StrokeCap, TextAlignH, TextAlignV, TextAutoResize, TextCase, TextDecoration,
    TextStyle, VConstraint, VariableType, VariableValue,
};
use serde_json::{Map, Value, json};

use super::{ROOT_ID, fingerprint, layout_fingerprint, strokes_fingerprint, text_fingerprint, tokens_fingerprint, uuid};
use crate::{FormatError, io_err, json_options};

const DEFAULT_FEATURES: &[&str] = &["fdata/path-data", "design-tokens/v1", "variants/v1", "layout/grid", "components/v2", "fdata/shape-data-type"];

const MIGRATIONS: &[&str] = &[
    "legacy-2",
    "legacy-3",
    "legacy-5",
    "legacy-6",
    "legacy-7",
    "legacy-8",
    "legacy-9",
    "legacy-10",
    "legacy-11",
    "legacy-12",
    "legacy-13",
    "legacy-14",
    "legacy-16",
    "legacy-17",
    "legacy-18",
    "legacy-19",
    "legacy-25",
    "legacy-26",
    "legacy-27",
    "legacy-28",
    "legacy-29",
    "legacy-31",
    "legacy-32",
    "legacy-33",
    "legacy-34",
    "legacy-36",
    "legacy-37",
    "legacy-38",
    "legacy-39",
    "legacy-40",
    "legacy-41",
    "legacy-42",
    "legacy-43",
    "legacy-44",
    "legacy-45",
    "legacy-46",
    "legacy-47",
    "legacy-48",
    "legacy-49",
    "legacy-50",
    "legacy-51",
    "legacy-52",
    "legacy-53",
    "legacy-54",
    "legacy-55",
    "legacy-56",
    "legacy-57",
    "legacy-59",
    "legacy-62",
    "legacy-65",
    "legacy-66",
    "legacy-67",
    "0001-remove-tokens-from-groups",
    "0002-normalize-bool-content-v2",
    "0002-clean-shape-interactions",
    "0003-fix-root-shape",
    "0003-convert-path-content-v2",
    "0005-deprecate-image-type",
    "0006-fix-old-texts-fills",
    "0008-fix-library-colors-v4",
    "0009-clean-library-colors",
    "0009-add-partial-text-touched-flags",
    "0010-fix-swap-slots-pointing-non-existent-shapes",
    "0011-fix-invalid-text-touched-flags",
    "0012-fix-position-data",
    "0013-fix-component-path",
    "0013-clear-invalid-strokes-and-fills",
    "0014-fix-tokens-lib-duplicate-ids",
    "0014-clear-components-nil-objects",
    "0015-fix-text-attrs-blank-strings",
    "0015-clean-shadow-color",
    "0016-copy-fills-from-position-data-to-text-node",
    "0017-fix-layout-flex-dir",
    "0018-remove-unneeded-objects-from-components",
    "0019-fix-missing-swap-slots",
    "0020-sync-component-id-with-near-main",
    "0021-fix-shape-svg-attrs",
    "0022-normalize-component-root-and-resync",
    "0023-repair-token-themes-with-inexistent-sets",
    "0024b-fix-stroke-cap-placement",
    "0025-repair-empty-text-content",
    "0026-fix-svg-raw-shapes-uuids",
];

const SOURCE_LAYOUT_KEYS: &[&str] = &[
    "layout",
    "layoutFlexDir",
    "layoutWrapType",
    "layoutGap",
    "layoutGapType",
    "layoutPadding",
    "layoutPaddingType",
    "layoutJustifyContent",
    "layoutJustifyItems",
    "layoutAlignItems",
    "layoutAlignContent",
    "layoutGridDir",
    "layoutGridColumns",
    "layoutGridRows",
    "layoutGridCells",
];

const PASSTHROUGH_KEYS: &[&str] = &["pluginData", "svgViewbox", "svgDefs", "layoutItemZIndex", "exports", "hideInViewer"];

struct Media {
    id: String,
    object: String,
    mtype: &'static str,
    ext: &'static str,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
}

struct Exporter<'a> {
    doc: &'a Document,
    file_id: String,
    page_ids: Vec<(NodeId, String)>,
    ids: BTreeMap<NodeId, String>,
    page_of: BTreeMap<NodeId, String>,
    component_ids: BTreeMap<NodeId, String>,
    media: BTreeMap<String, Media>,
    token_names: BTreeMap<String, String>,
    unchanged_layouts: BTreeSet<NodeId>,
    components: Vec<Value>,
    warnings: Vec<String>,
}

/// Write an offline Penpot 2.x archive and warnings for unsupported native features.
pub fn export(doc: &Document) -> Result<(Vec<u8>, Vec<String>), FormatError> {
    let mut exporter = Exporter::new(doc);
    exporter.assign_ids();
    exporter.write()
}

/// Derive a stable UUID-shaped identifier from a document-local seed.
fn derived_id(seed: &str) -> String {
    let hash = crate::image_hash(seed.as_bytes());
    let mut hex: Vec<char> = hash.chars().take(32).collect();
    if let Some(c) = hex.get_mut(12) {
        *c = '4';
    }
    if let Some(c) = hex.get_mut(16) {
        *c = match c.to_digit(16).unwrap_or(0) & 0x3 {
            0 => '8',
            1 => '9',
            2 => 'a',
            _ => 'b',
        };
    }
    let s: String = hex.into_iter().collect();
    let part = |range: std::ops::Range<usize>| s.get(range).unwrap_or("0");
    format!("{}-{}-{}-{}-{}", part(0..8), part(8..12), part(12..16), part(16..20), part(20..32))
}

/// Encode RGB as hexadecimal; alpha is exported in its separate opacity field.
fn hex(c: Color) -> String {
    let [r, g, b, _] = c.to_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Emit a finite JSON number rounded to six decimal places.
fn num(v: f64) -> Value {
    if !v.is_finite() {
        return json!(0);
    }
    let rounded = (v * 1e6).round() / 1e6;
    if rounded.fract() == 0.0 && rounded.abs() < 1e15 { json!(rounded as i64) } else { json!(rounded) }
}

/// Format a sanitized number for SVG path text.
fn num_text(v: f64) -> String {
    match num(v) {
        Value::Number(n) => n.to_string(),
        _ => "0".into(),
    }
}

/// Decode a preserved Penpot source field from layer metadata.
fn source_value(node: &Node, key: &str) -> Option<Value> {
    serde_json::from_str(node.plugin_data.get(&format!("penpot.source.{key}"))?).ok()
}

/// Check whether native state still matches its import-time fingerprint.
fn matches_fingerprint(node: &Node, key: &str, current: Option<String>) -> bool {
    current.is_some() && node.plugin_data.get(&format!("penpot.fp.{key}")) == current.as_ref()
}

/// Recover preserved content for a layer originally imported as raw SVG.
fn svg_raw_source(node: &Node) -> Option<Value> {
    node.plugin_data.contains_key("penpot.svgRawStatus").then(|| source_value(node, "content")).flatten()
}

/// Serialize finite path elements as Penpot commands, converting quadratics to cubics.
fn path_content(path: &BezPath) -> Value {
    let mut out = Vec::new();
    let mut last = Point::ZERO;
    let finite = |p: &Point| p.x.is_finite() && p.y.is_finite();
    for el in path.elements() {
        let points_ok = match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => finite(&p),
            PathEl::QuadTo(c, p) => finite(&c) && finite(&p),
            PathEl::CurveTo(c1, c2, p) => finite(&c1) && finite(&c2) && finite(&p),
            PathEl::ClosePath => true,
        };
        if !points_ok {
            continue;
        }
        let item = match *el {
            PathEl::MoveTo(p) => {
                last = p;
                json!({"command": "move-to", "params": {"x": num(p.x), "y": num(p.y)}})
            }
            PathEl::LineTo(p) => {
                last = p;
                json!({"command": "line-to", "params": {"x": num(p.x), "y": num(p.y)}})
            }
            PathEl::QuadTo(c, p) => {
                let c1 = last + (c - last) * (2.0 / 3.0);
                let c2 = p + (c - p) * (2.0 / 3.0);
                last = p;
                json!({"command": "curve-to", "params": {"x": num(p.x), "y": num(p.y), "c1x": num(c1.x), "c1y": num(c1.y), "c2x": num(c2.x), "c2y": num(c2.y)}})
            }
            PathEl::CurveTo(c1, c2, p) => {
                last = p;
                json!({"command": "curve-to", "params": {"x": num(p.x), "y": num(p.y), "c1x": num(c1.x), "c1y": num(c1.y), "c2x": num(c2.x), "c2y": num(c2.y)}})
            }
            PathEl::ClosePath => json!({"command": "close-path", "params": {}}),
        };
        out.push(item);
    }
    Value::Array(out)
}

/// Write the six affine coefficients using Penpot field names.
fn matrix(m: Affine) -> Value {
    let [a, b, c, d, e, f] = m.as_coeffs();
    json!({"a": num(a), "b": num(b), "c": num(c), "d": num(d), "e": num(e), "f": num(f)})
}

/// Write both rectangle dimensions and corner coordinates.
fn rect_json(x: f64, y: f64, w: f64, h: f64) -> Value {
    json!({"x": num(x), "y": num(y), "width": num(w), "height": num(h), "x1": num(x), "y1": num(y), "x2": num(x + w), "y2": num(y + h)})
}

/// Insert a token under its nested dotted-name path.
fn insert_token(map: &mut Map<String, Value>, path: &[&str], token: Value) {
    match path {
        [] => {}
        [leaf] => {
            map.insert((*leaf).to_owned(), token);
        }
        [head, rest @ ..] => {
            let entry = map.entry((*head).to_owned()).or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            if let Value::Object(next) = entry {
                insert_token(next, rest, token);
            }
        }
    }
}

/// Distinguish a full ellipse from arcs and hollow ellipse shapes.
fn is_full_ellipse(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Ellipse { start, sweep, inner } if start.abs() < 1e-9 && (sweep - std::f64::consts::TAU).abs() < 1e-9 && inner.abs() < 1e-9)
}

/// Choose the supported Penpot shape type or omit non-shape nodes.
fn penpot_type(node: &Node) -> Option<&'static str> {
    if svg_raw_source(node).is_some() {
        return Some("svg-raw");
    }
    Some(match &node.kind {
        NodeKind::Frame | NodeKind::Component { .. } | NodeKind::ComponentSet { .. } | NodeKind::Instance { .. } | NodeKind::Section { .. } => {
            "frame"
        }
        NodeKind::Group => "group",
        NodeKind::BooleanOp { .. } => "bool",
        NodeKind::Rectangle => "rect",
        kind @ NodeKind::Ellipse { .. } if is_full_ellipse(kind) => "circle",
        NodeKind::Ellipse { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. } | NodeKind::Line | NodeKind::Vector { .. } => "path",
        NodeKind::Text(_) => "text",
        NodeKind::Slice | NodeKind::Page { .. } => return None,
    })
}

/// Map supported blend modes to Penpot names.
fn blend_name(mode: BlendMode) -> Option<&'static str> {
    match mode.css() {
        "plus-darker" | "plus-lighter" => None,
        name => Some(name),
    }
}

/// Serialize gradient stops and unit-box handles, including radial width.
fn gradient_json(g: &Gradient) -> Value {
    let (fx, fy) = g.from;
    let (tx, ty) = g.to;
    let len2 = (tx - fx).powi(2) + (ty - fy).powi(2);
    let width =
        if g.kind == GradientKind::Linear || len2 <= 1e-12 { 1.0 } else { (((g.width.0 - fx).powi(2) + (g.width.1 - fy).powi(2)) / len2).sqrt() };
    let kind = if g.kind == GradientKind::Linear { "linear" } else { "radial" };
    let stops: Vec<Value> = g
        .sorted_stops()
        .iter()
        .map(|s| json!({"color": hex(s.color), "opacity": num(f64::from(s.color.a)), "offset": num(f64::from(s.position))}))
        .collect();
    json!({"type": kind, "startX": num(fx), "startY": num(fy), "endX": num(tx), "endY": num(ty), "width": num(width), "stops": stops})
}

/// Infer the numeric font weight from a native style name.
fn weight_of(style: &str) -> u16 {
    let s = style.to_ascii_lowercase().replace(['-', '_'], " ");
    for (needle, weight) in [
        ("extra light", 200),
        ("ultra light", 200),
        ("extra bold", 800),
        ("ultra bold", 800),
        ("semi bold", 600),
        ("demi bold", 600),
        ("extralight", 200),
        ("extrabold", 800),
        ("semibold", 600),
        ("thin", 100),
        ("light", 300),
        ("medium", 500),
        ("bold", 700),
        ("black", 900),
        ("heavy", 900),
    ] {
        if s.contains(needle) {
            return weight;
        }
    }
    400
}

/// Preserve an imported provider id or derive an offline family reference.
fn font_id(node: &Node, family: &str) -> String {
    for key in ["penpot.googleFontRefs", "penpot.customFontRefs"] {
        let refs = node.plugin_data.get(key).and_then(|raw| serde_json::from_str::<Value>(raw).ok());
        if let Some(id) = refs
            .as_ref()
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|r| r.get("family").and_then(Value::as_str) == Some(family))
            .and_then(|r| r.get("id").and_then(Value::as_str))
        {
            return id.to_owned();
        }
    }
    if family == "Source Sans Pro" {
        return "sourcesanspro".into();
    }
    let slug: String = family.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    format!("gfont-{slug}")
}

impl<'a> Exporter<'a> {
    fn new(doc: &'a Document) -> Self {
        let file_id = doc
            .pages
            .iter()
            .filter_map(|p| doc.node(*p)?.plugin_data.get("penpot.fileId"))
            .find(|id| uuid(id))
            .cloned()
            .unwrap_or_else(|| derived_id(&format!("file:{}", doc.name)));
        Exporter {
            doc,
            file_id,
            page_ids: Vec::new(),
            ids: BTreeMap::new(),
            page_of: BTreeMap::new(),
            component_ids: BTreeMap::new(),
            media: BTreeMap::new(),
            token_names: BTreeMap::new(),
            unchanged_layouts: doc
                .nodes()
                .filter(|n| n.plugin_data.contains_key("penpot.fp.layout") && matches_fingerprint(n, "layout", layout_fingerprint(doc, n.id)))
                .map(|n| n.id)
                .collect(),
            components: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self.warnings.len() < 100 && !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }

    fn node(&self, id: NodeId) -> Option<&'a Node> {
        self.doc.node(id)
    }

    fn exported_children(&self, node: &Node) -> Vec<NodeId> {
        let raw = svg_raw_source(node).is_some();
        let mut kids: Vec<NodeId> = node
            .children
            .iter()
            .copied()
            .filter(|c| self.node(*c).is_some_and(|n| penpot_type(n).is_some() && (!raw || n.plugin_data.contains_key("penpot.id"))))
            .collect();
        if node.layout.as_ref().is_some_and(|l| matches!(l.mode, LayoutMode::Horizontal | LayoutMode::Vertical)) {
            kids.reverse();
        }
        kids
    }

    fn assign_ids(&mut self) {
        let mut pages_used = BTreeSet::new();
        let mut component_used = BTreeSet::new();
        for (index, &page) in self.doc.pages.iter().enumerate() {
            let Some(page_node) = self.node(page) else { continue };
            let page_id = page_node
                .plugin_data
                .get("penpot.pageId")
                .filter(|id| uuid(id) && !pages_used.contains(*id))
                .cloned()
                .unwrap_or_else(|| derived_id(&format!("{}:page:{index}:{}", self.file_id, page.0)));
            pages_used.insert(page_id.clone());
            self.page_ids.push((page, page_id.clone()));
            let mut used = BTreeSet::from([ROOT_ID.to_owned()]);
            let mut stack: Vec<NodeId> = page_node.children.iter().rev().copied().collect();
            while let Some(id) = stack.pop() {
                let Some(node) = self.node(id) else { continue };
                if penpot_type(node).is_none() {
                    continue;
                }
                let mut pid = node.plugin_data.get("penpot.id").filter(|p| uuid(p) && !used.contains(*p)).cloned();
                let mut salt = 0;
                while pid.is_none() {
                    let candidate = derived_id(&format!("{}:{page_id}:{}:{salt}", self.file_id, id.0));
                    if !used.contains(&candidate) {
                        pid = Some(candidate);
                    }
                    salt += 1;
                }
                let pid = pid.unwrap_or_default();
                used.insert(pid.clone());
                self.ids.insert(id, pid);
                self.page_of.insert(id, page_id.clone());
                if matches!(node.kind, NodeKind::Component { .. }) {
                    let cid = node
                        .plugin_data
                        .get("penpot.componentMasterId")
                        .filter(|c| uuid(c) && !component_used.contains(*c))
                        .cloned()
                        .unwrap_or_else(|| derived_id(&format!("{}:component:{}", self.file_id, id.0)));
                    component_used.insert(cid.clone());
                    self.component_ids.insert(id, cid);
                }
                stack.extend(self.exported_children(node).into_iter().rev());
            }
        }
    }

    fn write(mut self) -> Result<(Vec<u8>, Vec<String>), FormatError> {
        let first_page = self.doc.pages.first().and_then(|p| self.node(*p));
        let header: Value = first_page.and_then(|p| source_value(p, "file")).unwrap_or(Value::Null);
        let features = header.get("features").cloned().unwrap_or_else(|| json!(DEFAULT_FEATURES));
        let migrations = if header.is_null() { Some(json!(MIGRATIONS)) } else { header.get("migrations").cloned() };
        let version = header.get("version").cloned().unwrap_or_else(|| json!(67));
        let options = header.get("options").cloned().unwrap_or_else(|| json!({"componentsV2": true, "baseFontSize": "16px"}));
        let name: String = self.doc.name.chars().take(256).collect();
        let base = format!("files/{}", self.file_id);
        let tokens = self.tokens_json();
        let mut entries: Vec<(String, Value)> = Vec::new();
        let mut file = json!({
            "id": self.file_id,
            "name": name,
            "features": features,
            "version": version,
            "revn": 1,
            "vern": 0,
            "isShared": header.get("isShared").cloned().unwrap_or(json!(false)),
            "hasMediaTrimmed": false,
            "options": options,
        });
        if let (Some(migrations), Some(obj)) = (migrations, file.as_object_mut()) {
            obj.insert("migrations".into(), migrations);
        }
        for key in ["createdAt", "modifiedAt"] {
            if let (Some(value), Some(obj)) = (header.get(key), file.as_object_mut()) {
                obj.insert(key.into(), value.clone());
            }
        }
        for (index, (page, page_id)) in self.page_ids.clone().into_iter().enumerate() {
            let Some(page_node) = self.node(page) else { continue };
            let mut objects: Vec<(String, Value)> = Vec::new();
            let top: Vec<NodeId> = page_node.children.iter().copied().filter(|c| self.ids.contains_key(c)).collect();
            let mut top_ids = Vec::new();
            for child in top {
                if let Some(id) = self.shape(child, ROOT_ID, ROOT_ID, &page_id, &mut objects) {
                    top_ids.push(json!(id));
                }
            }
            let background = match page_node.kind {
                NodeKind::Page { background, .. } => background,
                _ => Color::WHITE,
            };
            let mut root = self.base_shape(ROOT_ID, "Root Frame", "frame", 0.0, 0.0, 0.01, 0.01, Affine::IDENTITY);
            if let Some(obj) = root.as_object_mut() {
                obj.insert("parentId".into(), json!(ROOT_ID));
                obj.insert("frameId".into(), json!(ROOT_ID));
                obj.insert("pageId".into(), json!(page_id));
                obj.insert("fills".into(), json!([{"fillColor": hex(background), "fillOpacity": 1}]));
                obj.insert("strokes".into(), json!([]));
                obj.insert("shapes".into(), Value::Array(top_ids));
            }
            objects.push((ROOT_ID.into(), root));
            let page_json = self.page_json(page, &page_id, index, background);
            entries.push((format!("{base}/pages/{page_id}.json"), page_json));
            for (id, object) in objects {
                entries.push((format!("{base}/pages/{page_id}/{id}.json"), object));
            }
        }
        for component in std::mem::take(&mut self.components) {
            if let Some(id) = component.get("id").and_then(Value::as_str) {
                entries.push((format!("{base}/components/{id}.json"), component.clone()));
            }
        }
        let mut blobs: Vec<(String, Vec<u8>)> = Vec::new();
        for media in self.media.values() {
            entries.push((
                format!("{base}/media/{}.json", media.id),
                json!({"id": media.id, "name": "image", "width": media.width, "height": media.height, "mtype": media.mtype, "mediaId": media.object, "isLocal": true}),
            ));
            entries.push((
                format!("objects/{}.json", media.object),
                json!({"id": media.object, "size": media.bytes.len(), "contentType": media.mtype, "bucket": "file-media-object"}),
            ));
            blobs.push((format!("objects/{}.{}", media.object, media.ext), media.bytes.clone()));
        }
        let manifest = json!({
            "type": "penpot/export-files",
            "version": 1,
            "generatedBy": format!("figcraft/{}", env!("CARGO_PKG_VERSION")),
            "refer": "penpot",
            "files": [{"id": self.file_id, "name": name, "features": file.get("features").cloned().unwrap_or(Value::Null)}],
            "relations": [],
        });
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let mut put = |path: &str, bytes: &[u8]| -> Result<(), FormatError> {
                z.start_file(path, json_options()).map_err(io_err)?;
                z.write_all(bytes).map_err(io_err)
            };
            put("manifest.json", &serde_json::to_vec_pretty(&manifest).map_err(io_err)?)?;
            put(&format!("{base}.json"), &serde_json::to_vec_pretty(&file).map_err(io_err)?)?;
            if let Some(tokens) = tokens {
                put(&format!("{base}/tokens.json"), tokens.as_bytes())?;
            }
            for (path, value) in &entries {
                put(path, &serde_json::to_vec_pretty(value).map_err(io_err)?)?;
            }
            for (path, bytes) in &blobs {
                put(path, bytes)?;
            }
            z.finish().map_err(io_err)?;
        }
        Ok((buf.into_inner(), self.warnings))
    }

    fn page_json(&self, page: NodeId, page_id: &str, index: usize, background: Color) -> Value {
        let mut page_json = json!({"id": page_id, "name": self.node(page).map(|n| n.name.clone()).unwrap_or_default(), "index": index, "background": hex(background)});
        let Some(node) = self.node(page) else { return page_json };
        let NodeKind::Page { flows, .. } = &node.kind else { return page_json };
        let guides = &node.guides;
        let mut flow_map = Map::new();
        for (i, flow) in flows.iter().enumerate() {
            let Some(start) = self.ids.get(&flow.start) else { continue };
            let id = derived_id(&format!("{page_id}:flow:{i}:{}", flow.name));
            flow_map.insert(id.clone(), json!({"id": id, "name": flow.name, "startingFrame": start}));
        }
        let mut guide_map = Map::new();
        for (i, guide) in guides.iter().enumerate() {
            let id = derived_id(&format!("{page_id}:guide:{i}"));
            guide_map.insert(id.clone(), json!({"id": id, "axis": if guide.vertical { "x" } else { "y" }, "position": num(guide.offset)}));
        }
        let mut frame_guides = Vec::new();
        for (&nid, pid) in &self.ids {
            if self.page_of.get(&nid).map(String::as_str) != Some(page_id) {
                continue;
            }
            let Some(n) = self.node(nid) else { continue };
            if n.guides.is_empty() {
                continue;
            }
            let origin = self.doc.world_transform(nid) * Point::ZERO;
            for (i, guide) in n.guides.iter().enumerate() {
                frame_guides.push((
                    derived_id(&format!("{page_id}:{pid}:guide:{i}")),
                    guide.vertical,
                    if guide.vertical { origin.x } else { origin.y } + guide.offset,
                    pid.clone(),
                ));
            }
        }
        for (id, vertical, position, frame) in frame_guides {
            guide_map.insert(id.clone(), json!({"id": id, "axis": if vertical { "x" } else { "y" }, "position": num(position), "frameId": frame}));
        }
        if let Some(obj) = page_json.as_object_mut() {
            if !flow_map.is_empty() {
                obj.insert("flows".into(), Value::Object(flow_map));
            }
            if !guide_map.is_empty() {
                obj.insert("guides".into(), Value::Object(guide_map));
            }
        }
        page_json
    }

    fn base_shape(&self, id: &str, name: &str, typ: &str, x: f64, y: f64, w: f64, h: f64, linear: Affine) -> Value {
        let corners = [Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w, h), Point::new(0.0, h)];
        let center = Point::new(x + w / 2.0, y + h / 2.0);
        let world = Affine::translate(center.to_vec2()) * linear * Affine::translate((-w / 2.0, -h / 2.0));
        let points: Vec<Value> = corners
            .iter()
            .map(|p| {
                let q = world * *p;
                json!({"x": num(q.x), "y": num(q.y)})
            })
            .collect();
        let [a, b, ..] = linear.as_coeffs();
        let mut rotation = b.atan2(a).to_degrees();
        if rotation < 0.0 {
            rotation += 360.0;
        }
        if rotation.abs() < 1e-6 || (rotation - 360.0).abs() < 1e-6 {
            rotation = 0.0;
        }
        let inverse = if linear.determinant().abs() > 1e-12 { linear.inverse() } else { Affine::IDENTITY };
        let flipped = linear.determinant() < 0.0;
        json!({
            "id": id,
            "name": name,
            "type": typ,
            "x": num(x),
            "y": num(y),
            "width": num(w),
            "height": num(h),
            "rotation": num(rotation),
            "selrect": rect_json(x, y, w, h),
            "points": points,
            "transform": matrix(linear),
            "transformInverse": matrix(inverse),
            "flipX": if flipped { json!(true) } else { Value::Null },
            "flipY": Value::Null,
            "proportionLock": false,
        })
    }

    fn world_outline(&mut self, id: NodeId, depth: usize) -> BezPath {
        let Some(node) = self.node(id) else { return BezPath::new() };
        if depth > 64 {
            return BezPath::new();
        }
        match &node.kind {
            NodeKind::Group => {
                let mut out = BezPath::new();
                for child in node.children.clone() {
                    out.extend(self.world_outline(child, depth + 1));
                }
                out
            }
            NodeKind::BooleanOp { op } => {
                let op = *op;
                let operands: Vec<(BezPath, BoolOp)> = node
                    .children
                    .clone()
                    .into_iter()
                    .map(|c| {
                        let child_op = self.node(c).and_then(|n| n.bool_op).unwrap_or(op);
                        (figcraft_pathops::closed(&self.world_outline(c, depth + 1)), child_op)
                    })
                    .collect();
                match figcraft_pathops::boolean_ops(operands.iter().map(|(p, o)| (p, *o))) {
                    Ok(path) => path,
                    Err(_) => {
                        self.warn(format!("Boolean layer '{}' could not be flattened; its operands were exported as one path", node.name));
                        let mut out = BezPath::new();
                        for (p, _) in operands {
                            out.extend(p);
                        }
                        out
                    }
                }
            }
            NodeKind::Vector { network } if node.fills.is_empty() || !node.strokes.is_empty() => self.doc.world_transform(id) * network.stroke_path(),
            _ => match figcraft_doc::geometry::shape_path(node) {
                Some(p) => self.doc.world_transform(id) * p,
                None => BezPath::new(),
            },
        }
    }

    fn shape(&mut self, id: NodeId, parent: &str, frame: &str, page_id: &str, objects: &mut Vec<(String, Value)>) -> Option<String> {
        let node = self.node(id)?;
        let pid = self.ids.get(&id)?.clone();
        let typ = penpot_type(node)?;
        let world = self.doc.world_transform(id);
        let (w, h) = (node.width.max(0.01), node.height.max(0.01));
        let [a, b, c, d, ..] = world.as_coeffs();
        let linear = Affine::new([a, b, c, d, 0.0, 0.0]);
        let center = world * Point::new(w / 2.0, h / 2.0);
        let mut shape = self.base_shape(&pid, &node.name, typ, center.x - w / 2.0, center.y - h / 2.0, w, h, linear);
        if matches!(typ, "path" | "bool") {
            let path = self.world_outline(id, 0);
            if let Some(obj) = shape.as_object_mut() {
                obj.insert("content".into(), path_content(&path));
            }
        }
        let child_frame = if typ == "frame" { pid.clone() } else { frame.to_owned() };
        let kids = self.exported_children(node);
        let mut child_ids = Vec::new();
        for child in &kids {
            if let Some(cid) = self.shape(*child, &pid, &child_frame, page_id, objects) {
                child_ids.push(cid);
            }
        }
        let parent_node = node.parent.and_then(|p| self.node(p));
        let mut fields = Map::new();
        fields.insert("parentId".into(), json!(parent));
        fields.insert("frameId".into(), json!(frame));
        fields.insert("pageId".into(), json!(page_id));
        fields.insert("fills".into(), Value::Array(self.paints(&node.fills, false)));
        fields.insert("strokes".into(), Value::Array(self.strokes(node, typ)));
        appearance_fields(node, &mut fields);
        self.effects(node, &mut fields);
        if matches!(typ, "rect" | "frame") {
            for (i, key) in ["r1", "r2", "r3", "r4"].iter().enumerate() {
                fields.insert((*key).into(), num(node.corners.radii.get(i).copied().unwrap_or(0.0)));
            }
        }
        if matches!(typ, "frame" | "group" | "bool" | "svg-raw") {
            fields.insert("shapes".into(), json!(child_ids));
        }
        if typ == "bool"
            && let NodeKind::BooleanOp { op } = node.kind
        {
            fields.insert("boolType".into(), json!(bool_type(op)));
        }
        if typ == "frame" {
            fields.insert("showContent".into(), json!(!node.clip));
            fields.insert("hideFillOnExport".into(), json!(false));
            if !node.grids.is_empty() {
                fields.insert("grids".into(), self.grids(node));
            }
            if matches!(node.kind, NodeKind::ComponentSet { .. }) {
                fields.insert("isVariantContainer".into(), json!(true));
            }
            self.layout(id, node, &kids, &mut fields);
        }
        if typ == "group" {
            let first_is_mask = node.children.first().and_then(|c| self.node(*c)).is_some_and(|c| c.is_mask);
            if first_is_mask {
                fields.insert("maskedGroup".into(), json!(true));
            }
        }
        if node.is_mask && !parent_node.is_some_and(|p| matches!(p.kind, NodeKind::Group) && p.children.first() == Some(&id)) {
            self.warn(format!(
                "Mask '{}' is not the bottom layer of a group; Penpot has no equivalent, so it was exported as a plain layer",
                node.name
            ));
        }
        self.item_fields(node, parent_node, &mut fields);
        if typ == "text"
            && let NodeKind::Text(text) = &node.kind
        {
            self.text_fields(node, text, &mut fields);
        }
        if typ == "svg-raw"
            && let Some(content) = svg_raw_source(node)
        {
            fields.insert("content".into(), content);
        }
        if let Some(interactions) = self.interactions(id, node) {
            fields.insert("interactions".into(), interactions);
        }
        self.components(id, node, &pid, page_id, &mut fields);
        if let Some(tokens) = self.applied_tokens(node) {
            fields.insert("appliedTokens".into(), tokens);
        }
        for key in PASSTHROUGH_KEYS {
            if let Some(value) = source_value(node, key) {
                fields.insert((*key).into(), value);
            }
        }
        if let Some(attrs) = self.svg_attrs(node, typ) {
            fields.insert("svgAttrs".into(), attrs);
        }
        if let Some(obj) = shape.as_object_mut() {
            obj.extend(fields);
        }
        objects.push((pid.clone(), shape));
        Some(pid)
    }

    /// A layer's fields as an auto layout child: its own, and the source's while its parent's layout
    /// is unchanged.
    fn item_fields(&mut self, node: &Node, parent_node: Option<&Node>, fields: &mut Map<String, Value>) {
        if parent_node.and_then(|p| p.layout.as_ref()).is_some() || node.child_layout != figcraft_doc::ChildLayout::default() {
            self.child_layout(node, fields);
        }
        if let Some(parent) = parent_node
            && self.unchanged_layouts.contains(&parent.id)
        {
            for key in [
                "layoutItemHSizing",
                "layoutItemVSizing",
                "layoutItemAbsolute",
                "layoutItemMargin",
                "layoutItemMarginType",
                "layoutItemAlignSelf",
                "layoutItemMinW",
                "layoutItemMaxW",
                "layoutItemMinH",
                "layoutItemMaxH",
            ] {
                if let Some(value) = source_value(node, key) {
                    fields.insert(key.into(), value);
                }
            }
        }
    }

    /// A text layer's fields: its content (the source's while its text is unchanged) and how it grows;
    /// its fills are in the content.
    fn text_fields(&mut self, node: &Node, text: &figcraft_doc::TextData, fields: &mut Map<String, Value>) {
        fields.insert("fills".into(), json!([]));
        let unchanged = matches_fingerprint(node, "text", text_fingerprint(node));
        match (unchanged, source_value(node, "content")) {
            (true, Some(content)) => {
                fields.insert("content".into(), content);
                if let Some(position) = source_value(node, "positionData") {
                    fields.insert("positionData".into(), position);
                }
            }
            _ => {
                fields.insert("content".into(), self.text_content(node, text));
            }
        }
        fields.insert(
            "growType".into(),
            json!(match text.auto_resize {
                TextAutoResize::WidthAndHeight => "auto-width",
                TextAutoResize::Height => "auto-height",
                TextAutoResize::None | TextAutoResize::Truncate => "fixed",
            }),
        );
    }

    fn svg_attrs(&self, node: &Node, typ: &str) -> Option<Value> {
        let mut attrs = source_value(node, "svgAttrs").and_then(|v| v.as_object().cloned());
        if !matches!(typ, "rect" | "circle" | "path" | "bool") {
            return attrs.map(Value::Object);
        }
        let ancestor_fill =
            self.doc.ancestors(node.id).iter().filter_map(|a| self.node(*a)).take_while(|a| !a.kind.is_frame_like()).any(|a| !a.fills.is_empty());
        if attrs.is_none() && (!node.fills.is_empty() || !ancestor_fill) {
            return None;
        }
        let map = attrs.get_or_insert_with(Map::new);
        if let Some(Value::Object(style)) = map.get_mut("style") {
            style.remove("fill");
        }
        if node.fills.is_empty() {
            map.insert("fill".into(), json!("none"));
        } else {
            map.remove("fill");
        }
        attrs.map(Value::Object)
    }

    fn image_fill(&mut self, hash: &str, keep_aspect: bool) -> Option<Value> {
        if !self.media.contains_key(hash) {
            let data = self.doc.images.get(hash)?;
            let (mtype, ext, bytes) = match image::guess_format(&data.bytes) {
                Ok(image::ImageFormat::Png) => ("image/png", "png", data.bytes.to_vec()),
                Ok(image::ImageFormat::Jpeg) => ("image/jpeg", "jpg", data.bytes.to_vec()),
                Ok(image::ImageFormat::WebP) => ("image/webp", "webp", data.bytes.to_vec()),
                Ok(image::ImageFormat::Gif) => ("image/gif", "gif", data.bytes.to_vec()),
                _ => {
                    let decoded = image::load_from_memory(&data.bytes).ok()?;
                    let mut out = Cursor::new(Vec::new());
                    decoded.write_to(&mut out, image::ImageFormat::Png).ok()?;
                    ("image/png", "png", out.into_inner())
                }
            };
            self.media.insert(
                hash.to_owned(),
                Media {
                    id: derived_id(&format!("{}:media:{hash}", self.file_id)),
                    object: derived_id(&format!("{}:object:{hash}", self.file_id)),
                    mtype,
                    ext,
                    bytes,
                    width: data.width.max(1),
                    height: data.height.max(1),
                },
            );
        }
        let media = self.media.get(hash)?;
        Some(
            json!({"id": media.id, "width": media.width, "height": media.height, "mtype": media.mtype, "name": "image", "keepAspectRatio": keep_aspect}),
        )
    }

    fn paint_fields(&mut self, paint: &Paint, stroke: bool) -> Option<Map<String, Value>> {
        let (color_key, gradient_key, image_key, opacity_key) = if stroke {
            ("strokeColor", "strokeColorGradient", "strokeImage", "strokeOpacity")
        } else {
            ("fillColor", "fillColorGradient", "fillImage", "fillOpacity")
        };
        let mut out = Map::new();
        match &paint.kind {
            PaintKind::Solid { color } => {
                out.insert(color_key.into(), json!(hex(*color)));
                out.insert(opacity_key.into(), num(f64::from((paint.opacity * color.a).clamp(0.0, 1.0))));
            }
            PaintKind::Gradient(g) => {
                if !matches!(g.kind, GradientKind::Linear | GradientKind::Radial) {
                    self.warn("Angular and diamond gradients have no Penpot equivalent; exported as radial gradients");
                }
                out.insert(gradient_key.into(), gradient_json(g));
                out.insert(opacity_key.into(), num(f64::from(paint.opacity.clamp(0.0, 1.0))));
            }
            PaintKind::Image(img) => {
                let keep_aspect = !(img.scale_mode == ImageScaleMode::Crop && img.crop.is_none_or(|c| c == [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
                if matches!(img.scale_mode, ImageScaleMode::Fit | ImageScaleMode::Tile) {
                    self.warn("Fit and tiled image fills have no Penpot equivalent; exported as fill images");
                }
                let Some(image) = self.image_fill(&img.image, keep_aspect) else {
                    self.warn("An image fill whose image is missing from the document was left out");
                    return None;
                };
                out.insert(image_key.into(), image);
                out.insert(opacity_key.into(), num(f64::from(paint.opacity.clamp(0.0, 1.0))));
            }
            PaintKind::Pattern(_) => {
                self.warn("Pattern fills have no Penpot equivalent and were left out");
                return None;
            }
        }
        Some(out)
    }

    fn paints(&mut self, paints: &[Paint], stroke: bool) -> Vec<Value> {
        let mut out = Vec::new();
        for paint in paints.iter().rev() {
            if !paint.visible && !stroke {
                self.warn("Hidden fills have no Penpot equivalent and were left out");
                continue;
            }
            if let Some(mut fields) = self.paint_fields(paint, stroke) {
                if stroke && !paint.visible {
                    fields.insert("hidden".into(), json!(true));
                }
                out.push(Value::Object(fields));
            }
        }
        out
    }

    fn strokes(&mut self, node: &Node, typ: &str) -> Vec<Value> {
        if matches_fingerprint(node, "strokes", strokes_fingerprint(node))
            && let Some(Value::Array(source)) = source_value(node, "strokes")
            && !source.iter().any(|s| s.get("strokeImage").is_some())
        {
            return source;
        }
        let mut strokes = self.paints(&node.strokes, true);
        let props = &node.stroke;
        let w = props.weight;
        let near = |a: &[f64], b: &[f64]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        let style = if props.dashes.is_empty() {
            "solid"
        } else if near(&props.dashes, &[0.0, w + 5.0]) {
            "dotted"
        } else if near(&props.dashes, &[w + 5.0, w + 5.0, w + 1.0, w + 5.0]) {
            "mixed"
        } else {
            if !near(&props.dashes, &[w + 10.0, w + 10.0]) {
                self.warn("Custom dash patterns have no Penpot equivalent; exported as Penpot's dashed style");
            }
            "dashed"
        };
        let cap = |cap: StrokeCap, dotted: bool| -> Option<&'static str> {
            match cap {
                StrokeCap::Round if !dotted => Some("round"),
                StrokeCap::Square => Some("square"),
                StrokeCap::LineArrow => Some("line-arrow"),
                StrokeCap::TriangleArrow | StrokeCap::ReversedTriangle => Some("triangle-arrow"),
                StrokeCap::CircleFilled => Some("circle-marker"),
                StrokeCap::DiamondFilled => Some("diamond-marker"),
                _ => None,
            }
        };
        let open = typ == "path"
            && figcraft_doc::geometry::stroke_geometry(node).is_some_and(|p| !p.elements().iter().any(|e| matches!(e, PathEl::ClosePath)));
        let alignment = match props.align {
            StrokeAlign::Inside => "inner",
            StrokeAlign::Center => "center",
            StrokeAlign::Outside => "outer",
        };
        for stroke in &mut strokes {
            let Some(obj) = stroke.as_object_mut() else { continue };
            obj.insert("strokeWidth".into(), num(w));
            obj.insert("strokeAlignment".into(), json!(alignment));
            obj.insert("strokeStyle".into(), json!(style));
            if open {
                if let Some(c) = cap(props.cap_start, style == "dotted") {
                    obj.insert("strokeCapStart".into(), json!(c));
                }
                if let Some(c) = cap(props.cap_end, style == "dotted") {
                    obj.insert("strokeCapEnd".into(), json!(c));
                }
            }
        }
        if props.sides.is_some() && !strokes.is_empty() {
            self.warn("Per-side stroke widths have no Penpot equivalent; exported with one width");
        }
        strokes
    }

    fn effects(&mut self, node: &Node, fields: &mut Map<String, Value>) {
        let mut shadows = Vec::new();
        for (i, effect) in node.effects.iter().enumerate() {
            let id = derived_id(&format!("{}:{}:effect:{i}", self.file_id, node.id.0));
            match &effect.kind {
                EffectKind::DropShadow(s) | EffectKind::InnerShadow(s) => {
                    let style = if matches!(effect.kind, EffectKind::DropShadow(_)) { "drop-shadow" } else { "inner-shadow" };
                    shadows.push(json!({
                        "id": id,
                        "style": style,
                        "offsetX": num(s.offset.0),
                        "offsetY": num(s.offset.1),
                        "blur": num(s.radius),
                        "spread": num(s.spread),
                        "hidden": !effect.visible,
                        "color": {"color": hex(s.color), "opacity": num(f64::from(s.color.a))},
                    }));
                }
                EffectKind::LayerBlur(b) => {
                    fields.insert("blur".into(), json!({"id": id, "type": "layer-blur", "value": num(b.radius), "hidden": !effect.visible}));
                }
                EffectKind::BackgroundBlur(b) => {
                    fields.insert(
                        "backgroundBlur".into(),
                        json!({"id": id, "type": "background-blur", "value": num(b.radius), "hidden": !effect.visible}),
                    );
                }
                _ => self.warn("Noise, texture and glass effects have no Penpot equivalent and were left out"),
            }
        }
        if !shadows.is_empty() {
            fields.insert("shadow".into(), Value::Array(shadows));
        }
    }

    fn grids(&self, node: &Node) -> Value {
        let grids: Vec<Value> = node
            .grids
            .iter()
            .map(|g| {
                let color = json!({"color": hex(g.color), "opacity": num(f64::from(g.color.a))});
                match g.kind {
                    GridKind::Grid => json!({"type": "square", "display": g.visible, "params": {"size": num(g.size), "color": color}}),
                    GridKind::Columns | GridKind::Rows => {
                        let align = match g.align {
                            GridAlign::Stretch => "stretch",
                            GridAlign::Min => "left",
                            GridAlign::Center => "center",
                            GridAlign::Max => "right",
                        };
                        json!({
                            "type": if g.kind == GridKind::Columns { "column" } else { "row" },
                            "display": g.visible,
                            "params": {"size": g.count, "itemLength": num(g.size), "gutter": num(g.gutter), "margin": num(g.offset), "type": align, "color": color},
                        })
                    }
                }
            })
            .collect();
        Value::Array(grids)
    }

    fn layout(&mut self, id: NodeId, node: &Node, kids: &[NodeId], fields: &mut Map<String, Value>) {
        let unchanged = self.unchanged_layouts.contains(&id);
        if unchanged {
            let source: Vec<(String, Value)> = SOURCE_LAYOUT_KEYS.iter().filter_map(|k| Some(((*k).to_owned(), source_value(node, k)?))).collect();
            if source.iter().any(|(k, _)| k == "layout")
                && (node.layout.as_ref().is_none_or(|al| al.mode != LayoutMode::Grid) || source.iter().any(|(k, _)| k == "layoutGridCells"))
            {
                fields.extend(source);
                return;
            }
        }
        let Some(al) = &node.layout else { return };
        let (row_gap, column_gap) = match al.mode {
            LayoutMode::Horizontal => (al.cross_gap, al.gap),
            LayoutMode::Vertical => (al.gap, al.cross_gap),
            LayoutMode::Grid => (al.cross_gap, al.gap),
        };
        let [p1, p2, p3, p4] = al.padding;
        fields.insert("layoutGap".into(), json!({"rowGap": num(row_gap), "columnGap": num(column_gap)}));
        fields.insert("layoutGapType".into(), json!("multiple"));
        fields.insert("layoutPadding".into(), json!({"p1": num(p1), "p2": num(p2), "p3": num(p3), "p4": num(p4)}));
        fields.insert("layoutPaddingType".into(), json!("multiple"));
        let justify = match al.main_align {
            MainAlign::Min => "start",
            MainAlign::Center => "center",
            MainAlign::Max => "end",
            MainAlign::SpaceBetween => "space-between",
            MainAlign::SpaceAround => "space-around",
            MainAlign::SpaceEvenly => "space-evenly",
        };
        let align = match al.cross_align {
            CrossAlign::Min | CrossAlign::Baseline => "start",
            CrossAlign::Center => "center",
            CrossAlign::Max => "end",
        };
        fields.insert("layoutAlignItems".into(), json!(align));
        match al.mode {
            LayoutMode::Horizontal | LayoutMode::Vertical => {
                let dir = if al.mode == LayoutMode::Horizontal { "row" } else { "column" };
                fields.insert("layout".into(), json!("flex"));
                fields.insert("layoutFlexDir".into(), json!(dir));
                fields.insert("layoutWrapType".into(), json!(if al.wrap { "wrap" } else { "nowrap" }));
                fields.insert("layoutJustifyContent".into(), json!(justify));
                fields.insert(
                    "layoutAlignContent".into(),
                    json!(match al.content_align {
                        ContentAlign::Auto => match al.cross_align {
                            CrossAlign::Center => "center",
                            CrossAlign::Max => "end",
                            _ => "start",
                        },
                        ContentAlign::SpaceBetween => "space-between",
                    }),
                );
                if al.cross_align == CrossAlign::Baseline {
                    self.warn("Baseline alignment has no Penpot equivalent; exported as start");
                }
            }
            LayoutMode::Grid => {
                let track = |t: &GridTrack| match t.sizing {
                    Sizing::Fixed => json!({"type": "fixed", "value": num(t.value)}),
                    Sizing::Hug => json!({"type": "auto"}),
                    Sizing::Fill => json!({"type": "flex", "value": num(t.value.max(0.001))}),
                };
                let mut columns: Vec<Value> = al.column_tracks.iter().map(track).collect();
                let mut rows: Vec<Value> = al.row_tracks.iter().map(track).collect();
                columns.resize(al.grid_columns.max(1) as usize, json!({"type": "flex", "value": 1}));
                rows.resize(al.grid_rows.max(1) as usize, json!({"type": "flex", "value": 1}));
                fields.insert("layout".into(), json!("grid"));
                fields.insert("layoutGridDir".into(), json!("row"));
                fields.insert(
                    "layoutJustifyItems".into(),
                    json!(match al.main_align {
                        MainAlign::Center => "center",
                        MainAlign::Max => "end",
                        _ => "start",
                    }),
                );
                fields.insert("layoutJustifyContent".into(), json!("start"));
                fields.insert("layoutAlignContent".into(), json!("start"));
                let (cells, used_rows) = self.grid_cells(node, al, kids, columns.len());
                rows.resize(rows.len().max(used_rows), json!({"type": "flex", "value": 1}));
                fields.insert("layoutGridColumns".into(), Value::Array(columns));
                fields.insert("layoutGridRows".into(), Value::Array(rows));
                fields.insert("layoutGridCells".into(), cells);
            }
        }
    }

    fn grid_cells(&mut self, node: &Node, al: &figcraft_doc::AutoLayout, kids: &[NodeId], columns: usize) -> (Value, usize) {
        let frame_id = self.ids.get(&node.id).cloned().unwrap_or_default();
        let flow: Vec<NodeId> =
            node.children.iter().copied().filter(|c| kids.contains(c) && self.node(*c).is_some_and(|n| !n.child_layout.absolute)).collect();
        let (placed, rows) = figcraft_layout::grid_place(self.doc, al, &flow);
        let mut taken = vec![vec![false; columns]; rows];
        let mut map = Map::new();
        let cell_align = |a: Option<CellAlign>| match a {
            Some(CellAlign::Min) => "start",
            Some(CellAlign::Center) => "center",
            Some(CellAlign::Max) => "end",
            None => "auto",
        };
        for cell in &placed {
            for row in taken.iter_mut().skip(cell.row).take(cell.row_span) {
                for slot in row.iter_mut().skip(cell.column).take(cell.column_span) {
                    *slot = true;
                }
            }
            let Some(n) = self.node(cell.id) else { continue };
            let manual = n.child_layout.grid_row.is_some() && n.child_layout.grid_column.is_some();
            let id = derived_id(&format!("{frame_id}:cell:{}:{}", cell.row, cell.column));
            map.insert(
                id.clone(),
                json!({
                    "id": id,
                    "row": cell.row + 1,
                    "rowSpan": cell.row_span,
                    "column": cell.column + 1,
                    "columnSpan": cell.column_span,
                    "position": if manual { "manual" } else { "auto" },
                    "alignSelf": cell_align(n.child_layout.grid_align_v),
                    "justifySelf": cell_align(n.child_layout.grid_align_h),
                    "shapes": [self.ids.get(&cell.id)],
                }),
            );
        }
        for (r, row) in taken.iter().enumerate() {
            for (c, used) in row.iter().enumerate() {
                if !used {
                    let id = derived_id(&format!("{frame_id}:cell:{r}:{c}"));
                    map.insert(
                        id.clone(),
                        json!({"id": id, "row": r + 1, "rowSpan": 1, "column": c + 1, "columnSpan": 1, "position": "auto", "alignSelf": "auto", "justifySelf": "auto", "shapes": []}),
                    );
                }
            }
        }
        (Value::Object(map), rows)
    }

    fn child_layout(&mut self, node: &Node, fields: &mut Map<String, Value>) {
        let cl = &node.child_layout;
        let sizing = |s: Sizing| match s {
            Sizing::Fixed => "fix",
            Sizing::Hug => "auto",
            Sizing::Fill => "fill",
        };
        fields.insert("layoutItemHSizing".into(), json!(sizing(cl.sizing_h)));
        fields.insert("layoutItemVSizing".into(), json!(sizing(cl.sizing_v)));
        if cl.absolute {
            fields.insert("layoutItemAbsolute".into(), json!(true));
        }
        for (key, value) in [("layoutItemMinW", cl.min_w), ("layoutItemMaxW", cl.max_w), ("layoutItemMinH", cl.min_h), ("layoutItemMaxH", cl.max_h)] {
            if let Some(v) = value {
                fields.insert(key.into(), num(v));
            }
        }
    }

    fn text_attrs(&mut self, node: &Node, style: &TextStyle, align: &str) -> Map<String, Value> {
        let weight = weight_of(&style.font_style);
        let italic = style.font_style.to_ascii_lowercase().contains("italic") || style.font_style.to_ascii_lowercase().contains("oblique");
        let variant = match (weight, italic) {
            (400, false) => "regular".to_owned(),
            (400, true) => "italic".to_owned(),
            (w, true) => format!("{w}italic"),
            (w, false) => w.to_string(),
        };
        let line_height = match style.line_height {
            LineHeight::Percent(p) => p / 100.0,
            LineHeight::Pixels(px) if style.font_size > 0.0 => px / style.font_size,
            _ => 1.2,
        };
        let letter_spacing = match style.letter_spacing {
            LetterSpacing::Pixels(px) => px,
            LetterSpacing::Percent(p) => p * style.font_size / 100.0,
        };
        let fills = match &style.fills {
            Some(fills) => self.paints(fills, false),
            None => self.paints(&node.fills, false),
        };
        let mut out = Map::new();
        out.insert("fontFamily".into(), json!(style.font_family));
        out.insert("fontId".into(), json!(font_id(node, &style.font_family)));
        out.insert("fontVariantId".into(), json!(variant));
        out.insert("fontSize".into(), json!(num_text(style.font_size)));
        out.insert("fontWeight".into(), json!(weight.to_string()));
        out.insert("fontStyle".into(), json!(if italic { "italic" } else { "normal" }));
        out.insert("lineHeight".into(), json!(num_text(line_height)));
        out.insert("letterSpacing".into(), json!(num_text(letter_spacing)));
        out.insert(
            "textDecoration".into(),
            json!(match style.decoration {
                TextDecoration::None => "none",
                TextDecoration::Underline => "underline",
                TextDecoration::Strikethrough => "line-through",
            }),
        );
        out.insert(
            "textTransform".into(),
            json!(match style.case {
                TextCase::Upper => "uppercase",
                TextCase::Lower => "lowercase",
                TextCase::Title => "capitalize",
                _ => "none",
            }),
        );
        out.insert("textAlign".into(), json!(align));
        out.insert("fills".into(), Value::Array(fills));
        out
    }

    fn text_content(&mut self, node: &Node, text: &figcraft_doc::TextData) -> Value {
        let align = match text.align_h {
            TextAlignH::Left => "left",
            TextAlignH::Center => "center",
            TextAlignH::Right => "right",
            TextAlignH::Justified => "justify",
        };
        let chars: Vec<char> = text.characters.chars().collect();
        let style_at = |i: usize| -> &TextStyle { text.runs.iter().rev().find(|r| r.start <= i && i < r.end).map_or(&text.style, |r| &r.style) };
        let mut paragraphs = Vec::new();
        let mut start = 0usize;
        loop {
            let end = (start..chars.len()).find(|&i| chars.get(i) == Some(&'\n')).unwrap_or(chars.len());
            let mut leaves = Vec::new();
            let mut i = start;
            while i < end {
                let style = style_at(i);
                let mut j = i + 1;
                while j < end && style_at(j) == style {
                    j += 1;
                }
                let mut leaf = self.text_attrs(node, style, align);
                leaf.insert("text".into(), json!(chars.get(i..j).map(|s| s.iter().collect::<String>()).unwrap_or_default()));
                leaves.push(Value::Object(leaf));
                i = j;
            }
            let paragraph_style = if end > start { style_at(start) } else { style_at(start.saturating_sub(1)) };
            let mut paragraph = self.text_attrs(node, paragraph_style, align);
            if leaves.is_empty() {
                let mut leaf = paragraph.clone();
                leaf.insert("text".into(), json!(""));
                leaves.push(Value::Object(leaf));
            }
            paragraph.insert("type".into(), json!("paragraph"));
            paragraph.insert("children".into(), Value::Array(leaves));
            paragraphs.push(Value::Object(paragraph));
            if end >= chars.len() {
                break;
            }
            start = end + 1;
        }
        let mut root = Value::Object(self.text_attrs(node, &text.style, align));
        if let Some(obj) = root.as_object_mut() {
            obj.remove("fills");
            obj.insert("type".into(), json!("root"));
            obj.insert("children".into(), json!([{"type": "paragraph-set", "children": paragraphs}]));
        }
        let vertical = match text.align_v {
            TextAlignV::Top => None,
            TextAlignV::Middle => Some("center"),
            TextAlignV::Bottom => Some("bottom"),
        };
        if let (Some(v), Some(obj)) = (vertical, root.as_object_mut()) {
            obj.insert("verticalAlign".into(), json!(v));
        }
        root
    }

    fn interactions(&mut self, id: NodeId, node: &Node) -> Option<Value> {
        if let Some(source) = source_value(node, "interactions")
            && matches_fingerprint(node, "interactions", fingerprint(&node.interactions))
        {
            let exported: BTreeSet<&String> = self.ids.values().collect();
            let resolves = source.as_array().into_iter().flatten().all(|i| {
                ["destination", "positionRelativeTo"]
                    .iter()
                    .all(|k| i.get(*k).and_then(Value::as_str).is_none_or(|d| exported.contains(&d.to_owned())))
            });
            if resolves {
                return Some(source);
            }
        }
        if node.interactions.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for interaction in &node.interactions {
            for action in &interaction.actions {
                if let Some(item) = self.interaction(id, node, interaction, action) {
                    out.push(item);
                }
            }
        }
        Some(Value::Array(out))
    }

    fn interaction(&mut self, id: NodeId, node: &Node, interaction: &Interaction, action: &Action) -> Option<Value> {
        let event = match interaction.trigger {
            Trigger::OnClick | Trigger::MouseUp => "click",
            Trigger::OnPress | Trigger::MouseDown => "mouse-press",
            Trigger::OnHover => "mouse-over",
            Trigger::MouseEnter => "mouse-enter",
            Trigger::MouseLeave => "mouse-leave",
            Trigger::AfterDelay { .. } => "after-delay",
            _ => {
                self.warn(format!("Prototype trigger on '{}' has no Penpot equivalent and was left out", node.name));
                return None;
            }
        };
        let mut out = Map::new();
        out.insert("eventType".into(), json!(event));
        if let Trigger::AfterDelay { ms } = interaction.trigger {
            out.insert("delay".into(), json!(ms));
        }
        let destination = |dest: &NodeId, exporter: &mut Self| -> Option<String> {
            let found = exporter.ids.get(dest).cloned();
            if found.is_none() {
                exporter.warn(format!("A prototype link on '{}' points at a layer that is not exported; it was left out", node.name));
            }
            found
        };
        let overlay = |position: &OverlayPosition| match position {
            OverlayPosition::Manual => "manual",
            OverlayPosition::Center => "center",
            OverlayPosition::TopLeft => "top-left",
            OverlayPosition::TopRight => "top-right",
            OverlayPosition::TopCenter => "top-center",
            OverlayPosition::BottomLeft => "bottom-left",
            OverlayPosition::BottomRight => "bottom-right",
            OverlayPosition::BottomCenter => "bottom-center",
        };
        let is_overlay = matches!(action, Action::OpenOverlay { .. });
        match action {
            Action::Navigate { dest, preserve_scroll } => {
                out.insert("actionType".into(), json!("navigate"));
                out.insert("destination".into(), json!(destination(dest, self)?));
                out.insert("preserveScroll".into(), json!(preserve_scroll));
            }
            Action::OpenOverlay { dest, position, close_on_outside, background, offset, anchor } => {
                if *anchor != figcraft_doc::proto::OverlayAnchor::Screen {
                    self.warn(format!("Overlay anchor on '{}' was approximated relative to the screen", node.name));
                }
                out.insert("actionType".into(), json!("open-overlay"));
                out.insert("destination".into(), json!(destination(dest, self)?));
                out.insert("overlayPosType".into(), json!(overlay(position)));
                let [x, y] = offset.unwrap_or([0.0, 0.0]);
                out.insert("overlayPosition".into(), json!({"x": num(x), "y": num(y)}));
                out.insert("closeClickOutside".into(), json!(close_on_outside));
                out.insert("backgroundOverlay".into(), json!(background.is_some()));
                if *position == OverlayPosition::Manual
                    && let Some(relative) = self.ids.get(&id)
                {
                    out.insert("positionRelativeTo".into(), json!(relative));
                }
            }
            Action::CloseOverlay => {
                out.insert("actionType".into(), json!("close-overlay"));
            }
            Action::Back => {
                out.insert("actionType".into(), json!("prev-screen"));
            }
            Action::OpenUrl { url } => {
                out.insert("actionType".into(), json!("open-url"));
                out.insert("url".into(), json!(url));
            }
            _ => {
                self.warn(format!("A prototype action on '{}' has no Penpot equivalent and was left out", node.name));
                return None;
            }
        }
        if !matches!(action, Action::Back | Action::OpenUrl { .. })
            && let Some(animation) = self.animation(interaction, is_overlay)
        {
            out.insert("animation".into(), animation);
        }
        Some(Value::Object(out))
    }

    fn animation(&mut self, interaction: &Interaction, overlay: bool) -> Option<Value> {
        let easing = match interaction.easing {
            Easing::Linear => "linear",
            Easing::EaseIn => "ease-in",
            Easing::EaseOut => "ease-out",
            Easing::EaseInOut => "ease-in-out",
            _ => "ease",
        };
        let direction = |d: Direction| match d {
            Direction::Left => "left",
            Direction::Right => "right",
            Direction::Top => "up",
            Direction::Bottom => "down",
        };
        let duration = interaction.duration_ms;
        Some(match interaction.transition {
            Transition::Instant => return None,
            Transition::Dissolve => json!({"animationType": "dissolve", "duration": duration, "easing": easing}),
            Transition::SmartAnimate => {
                self.warn("Smart animate has no Penpot equivalent; exported as dissolve");
                json!({"animationType": "dissolve", "duration": duration, "easing": easing})
            }
            Transition::SlideIn { direction: d } | Transition::MoveIn { direction: d } => {
                json!({"animationType": "slide", "duration": duration, "easing": easing, "way": "in", "direction": direction(d), "offsetEffect": false})
            }
            Transition::SlideOut { direction: d } | Transition::MoveOut { direction: d } => {
                json!({"animationType": "slide", "duration": duration, "easing": easing, "way": if overlay { "in" } else { "out" }, "direction": direction(d), "offsetEffect": false})
            }
            Transition::Push { direction: d } if !overlay => {
                json!({"animationType": "push", "duration": duration, "easing": easing, "direction": direction(d)})
            }
            Transition::Push { direction: d } => {
                json!({"animationType": "slide", "duration": duration, "easing": easing, "way": "in", "direction": direction(d), "offsetEffect": false})
            }
        })
    }

    fn components(&mut self, id: NodeId, node: &Node, pid: &str, page_id: &str, fields: &mut Map<String, Value>) {
        let inside_instance = self.doc.ancestors(id).iter().any(|a| self.node(*a).is_some_and(|n| matches!(n.kind, NodeKind::Instance { .. })));
        let inside_main = self.doc.ancestors(id).iter().any(|a| self.node(*a).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. })));
        match &node.kind {
            NodeKind::Component { .. } => {
                let Some(cid) = self.component_ids.get(&id).cloned() else { return };
                fields.insert("componentId".into(), json!(cid));
                fields.insert("componentFile".into(), json!(self.file_id));
                fields.insert("componentRoot".into(), json!(true));
                fields.insert("mainInstance".into(), json!(true));
                let source = source_value(node, "component").and_then(|v| v.as_object().cloned()).unwrap_or_default();
                let parent = node.parent.and_then(|p| self.node(p));
                let mut component = Map::new();
                component.insert("id".into(), json!(cid));
                component.insert("name".into(), source.get("name").cloned().unwrap_or_else(|| json!(node.name)));
                component.insert("path".into(), source.get("path").cloned().unwrap_or_else(|| json!("")));
                component.insert("mainInstanceId".into(), json!(pid));
                component.insert("mainInstancePage".into(), json!(page_id));
                if let Some(set) = parent.filter(|p| matches!(p.kind, NodeKind::ComponentSet { .. }))
                    && let Some(set_id) = self.ids.get(&set.id)
                {
                    let properties: Vec<(String, String)> = node
                        .name
                        .split(',')
                        .filter_map(|pair| {
                            let (k, v) = pair.split_once('=')?;
                            Some((k.trim().to_owned(), v.trim().to_owned()))
                        })
                        .filter(|(k, _)| !k.is_empty())
                        .collect();
                    if !source.contains_key("name") {
                        component.insert("name".into(), json!(set.name));
                    }
                    component.insert("variantId".into(), json!(set_id));
                    component.insert(
                        "variantProperties".into(),
                        json!(properties.iter().map(|(k, v)| json!({"name": k, "value": v})).collect::<Vec<_>>()),
                    );
                    fields.insert("variantId".into(), json!(set_id));
                    fields.insert("variantName".into(), json!(properties.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>().join(", ")));
                    if let Some(original) = node.plugin_data.get("penpot.originalName") {
                        fields.insert("name".into(), json!(original));
                    }
                }
                self.components.push(Value::Object(component));
            }
            NodeKind::Instance { component, .. } => {
                let Some(cid) = self.component_ids.get(component).cloned() else {
                    self.warn(format!("Instance '{}' points at a component that is not exported; it was exported as a plain board", node.name));
                    return;
                };
                fields.insert("componentId".into(), json!(cid));
                fields.insert("componentFile".into(), json!(self.file_id));
                if !inside_instance && !inside_main {
                    fields.insert("componentRoot".into(), json!(true));
                }
                let reference = node.source.unwrap_or(*component);
                if let Some(r) = self.ids.get(&reference) {
                    fields.insert("shapeRef".into(), json!(r));
                }
            }
            _ => {
                if inside_instance && let Some(r) = node.source.and_then(|s| self.ids.get(&s)) {
                    fields.insert("shapeRef".into(), json!(r));
                } else if !inside_instance {
                    self.unlinked_copy(node, fields);
                }
            }
        }
        if inside_instance || matches!(node.kind, NodeKind::Instance { .. }) {
            let mut touched: BTreeSet<String> = source_value(node, "touched")
                .and_then(|v| v.as_array().cloned())
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            for group in &node.overrides {
                let mapped = match group.as_str() {
                    "fills" => "fill-group",
                    "strokes" | "strokeWeight" | "stroke" => "stroke-group",
                    "characters" | "text" | "textStyle" => "content-group",
                    "visible" => "visibility-group",
                    "name" => "name-group",
                    "effects" => "shadow-group",
                    "opacity" | "blend" => "layer-group",
                    "cornerRadius" | "corners" => "radius-group",
                    "width" | "height" | "size" | "transform" => "geometry-group",
                    _ => continue,
                };
                touched.insert(mapped.into());
            }
            if !touched.is_empty() {
                fields.insert("touched".into(), json!(touched));
            }
        }
    }

    fn unlinked_copy(&mut self, node: &Node, fields: &mut Map<String, Value>) {
        let component = node.plugin_data.get("penpot.componentId").filter(|c| self.component_ids.values().any(|v| v == *c));
        let reference = node.plugin_data.get("penpot.shapeRef").filter(|r| self.ids.values().any(|v| v == *r));
        if let Some(component) = component {
            fields.insert("componentId".into(), json!(component));
            fields.insert("componentFile".into(), json!(self.file_id));
            match source_value(node, "componentRoot") {
                Some(root) => {
                    fields.insert("componentRoot".into(), root);
                }
                None if node.plugin_data.contains_key("penpot.componentLinkStatus") => {
                    fields.insert("componentRoot".into(), json!(true));
                }
                None => {}
            }
        }
        if let Some(reference) = reference {
            fields.insert("shapeRef".into(), json!(reference));
            if let Some(touched) = source_value(node, "touched") {
                fields.insert("touched".into(), touched);
            }
        }
        if node.plugin_data.contains_key("penpot.componentId") && component.is_none() {
            self.warn("Some component copies point at components that are not in the exported file; they were exported as plain layers");
        }
    }

    fn token_name(&mut self, variable: &str) -> Option<String> {
        if let Some(name) = self.token_names.get(variable) {
            return Some(name.clone());
        }
        let var = self.doc.variables.get_by_id(variable)?;
        let name = if variable.starts_with("penpot:tokens:") {
            var.name.split_once('/').map_or(var.name.clone(), |(_, n)| n.to_owned())
        } else {
            var.name.split('/').map(|s| s.trim().replace(' ', "-")).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(".")
        };
        self.token_names.insert(variable.to_owned(), name.clone());
        Some(name)
    }

    fn applied_tokens(&mut self, node: &Node) -> Option<Value> {
        let mut out = source_value(node, "appliedTokens").and_then(|v| v.as_object().cloned()).unwrap_or_default();
        let vertical = node.layout.as_ref().is_some_and(|l| l.mode == LayoutMode::Vertical);
        for (field, variable) in &node.bound {
            let attr = match field.as_str() {
                "fills/0/color" => "fill",
                "strokes/0/color" => "strokeColor",
                "width" => "width",
                "height" => "height",
                "paddingTop" => "p1",
                "paddingRight" => "p2",
                "paddingBottom" => "p3",
                "paddingLeft" => "p4",
                "gap" if vertical => "rowGap",
                "gap" => "columnGap",
                "crossGap" if vertical => "columnGap",
                "crossGap" => "rowGap",
                "topLeftRadius" => "r1",
                "topRightRadius" => "r2",
                "bottomRightRadius" => "r3",
                "bottomLeftRadius" => "r4",
                "strokeWeight" => "strokeWidth",
                "fontSize" => "fontSize",
                "letterSpacing" => "letterSpacing",
                "fontFamily" => "fontFamily",
                _ => continue,
            };
            if let Some(name) = self.token_name(variable) {
                out.insert(attr.into(), json!(name));
            }
        }
        (!out.is_empty()).then_some(Value::Object(out))
    }

    /// The archive's `tokens.json`: the file's own Penpot tokens when it came from Penpot and its
    /// variables are unchanged since, else one token set per collection (per mode, with a theme
    /// for each, when it has several). Aliases go out as `{name}` references and colour
    /// calculations as the colour they show; a token that resolves to nothing is left out with a
    /// warning, so no reference may name it.
    fn tokens_json(&mut self) -> Option<String> {
        let first = self.doc.pages.first().and_then(|p| self.node(*p));
        let source = first.and_then(|p| p.plugin_data.iter().find(|(k, _)| k.starts_with("penpot.tokens.")).map(|(_, v)| v.clone()));
        if let Some(source) = source.filter(|s| serde_json::from_str::<Value>(s).is_ok()) {
            if first.and_then(|p| p.plugin_data.get("penpot.fp.tokens")) != tokens_fingerprint(self.doc).as_ref() {
                self.warn("Variables edited since the Penpot import are not exported; the file keeps its original Penpot tokens");
            }
            return Some(source);
        }
        let doc = self.doc;
        let variables = &doc.variables;
        if variables.variables.is_empty() {
            return None;
        }
        let flattener = variables.flattener();
        let mut root = Map::new();
        let mut themes = Vec::new();
        let mut order = Vec::new();
        let mut active_themes = Vec::new();
        let mut active_sets = Vec::new();
        for collection in &variables.collections {
            let many = collection.modes.len() > 1;
            for (index, mode) in collection.modes.iter().enumerate() {
                let set_name = if many { format!("{}/{}", collection.name, mode.name) } else { collection.name.clone() };
                let mut tokens = Map::new();
                for var in variables.variables.iter().filter(|v| v.collection == collection.id) {
                    let Some(value) = var.value_or_first(&mode.id) else { continue };
                    // Penpot tokens have no shape for colour calculations: the colour they resolve
                    // to in this mode goes out as a plain colour token, and so does an alias to one
                    // some set leaves out (its `{name}` would dangle whenever that set is active).
                    let Some(value) = flattener.value(var, &mode.id, value) else {
                        self.warn(crate::CALCULATIONS_LEFT_OUT);
                        continue;
                    };
                    let (kind, value) = match &value {
                        VariableValue::Alias(target) => {
                            let Some(target_name) = self.token_name(target) else { continue };
                            let kind = match var.kind {
                                VariableType::Color => "color",
                                VariableType::Number => "number",
                                VariableType::String => "string",
                                VariableType::Boolean => "boolean",
                            };
                            (kind, json!(format!("{{{target_name}}}")))
                        }
                        VariableValue::Color(c) => {
                            let [r, g, b, a] = c.to_rgba8();
                            ("color", json!(if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }))
                        }
                        // `flattened` leaves none: this format has no shape for a calculation.
                        VariableValue::ColorAlpha { .. } | VariableValue::ColorMix { .. } => continue,
                        VariableValue::Number(n) => ("number", json!(num_text(*n))),
                        VariableValue::String(s) => ("string", json!(s)),
                        VariableValue::Boolean(b) => ("boolean", json!(b.to_string())),
                    };
                    let Some(name) = self.token_name(&var.id) else { continue };
                    let parts: Vec<&str> = name.split('.').collect();
                    insert_token(&mut tokens, &parts, json!({"$value": value, "$type": kind, "$description": var.description}));
                }
                root.insert(set_name.clone(), Value::Object(tokens));
                order.push(set_name.clone());
                if many {
                    let mut selected = Map::new();
                    selected.insert(set_name.clone(), json!("enabled"));
                    themes.push(json!({
                        "id": derived_id(&format!("{}:theme:{}:{}", self.file_id, collection.id, mode.id)),
                        "name": mode.name,
                        "group": collection.name,
                        "description": "",
                        "isSource": false,
                        "selectedTokenSets": selected,
                    }));
                    if index == 0 {
                        active_themes.push(format!("{}/{}", collection.name, mode.name));
                        active_sets.push(set_name);
                    }
                } else {
                    active_sets.push(set_name);
                }
            }
        }
        root.insert("$themes".into(), Value::Array(themes));
        root.insert("$metadata".into(), json!({"tokenSetOrder": order, "activeThemes": active_themes, "activeSets": active_sets}));
        serde_json::to_string_pretty(&Value::Object(root)).ok()
    }
}

/// A layer's visibility, lock, opacity, blend, proportion lock and constraints as Penpot fields.
fn appearance_fields(node: &Node, fields: &mut Map<String, Value>) {
    if !node.visible {
        fields.insert("hidden".into(), json!(true));
    }
    if node.locked {
        fields.insert("blocked".into(), json!(true));
    }
    if (node.opacity - 1.0).abs() > 1e-6 {
        fields.insert("opacity".into(), num(f64::from(node.opacity)));
    }
    if let Some(name) = blend_name(node.blend).filter(|n| *n != "normal") {
        fields.insert("blendMode".into(), json!(name));
    }
    fields.insert("proportionLock".into(), json!(node.constrain_proportions));
    fields.insert(
        "constraintsH".into(),
        json!(match node.constraints.horizontal {
            HConstraint::Left => "left",
            HConstraint::Right => "right",
            HConstraint::LeftRight => "leftright",
            HConstraint::Center => "center",
            HConstraint::Scale => "scale",
        }),
    );
    fields.insert(
        "constraintsV".into(),
        json!(match node.constraints.vertical {
            VConstraint::Top => "top",
            VConstraint::Bottom => "bottom",
            VConstraint::TopBottom => "topbottom",
            VConstraint::Center => "center",
            VConstraint::Scale => "scale",
        }),
    );
}

/// A boolean operation's Penpot `boolType`.
fn bool_type(op: BoolOp) -> &'static str {
    match op {
        BoolOp::Union => "union",
        BoolOp::Subtract => "difference",
        BoolOp::Intersect => "intersection",
        BoolOp::Exclude => "exclude",
    }
}
