//! Native, offline importer for Penpot 2.x `.penpot` exports (ZIP / JSON).
//!
//! The archive contains `manifest.json`, one JSON per page and shape, and image blobs in
//! `objects/`. Imported shapes are ordinary FigCraft nodes, not a flattened SVG.
//! Only the supported subset is mapped; unknown properties are not silently represented as
//! fully editable. This module has no Python, network or UI dependency and works with WASM.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Cursor, Read};

use figcraft_doc::color::{BlendMode, Color, ColorStop, Gradient, GradientKind, ImageScaleMode, Paint, PaintKind};
use figcraft_doc::geom::{Affine, BezPath, Shape};
use figcraft_doc::{
    AutoLayout, Blur, BoolOp, CellAlign, Collection, ContentAlign, CrossAlign, Document, Effect, EffectKind, GridAlign, GridKind, GridTrack, Guide,
    HConstraint, LayoutGrid, LayoutMode, LetterSpacing, LineHeight, MainAlign, MaskType, Mode, NodeId, NodeKind, Shadow, Sizing, StrokeAlign,
    StrokeCap, StrokeJoin, Style, StyleKind, StyleRun, TextAlignH, TextAutoResize, TextCase, TextData, TextDecoration, TextStyle, VConstraint,
    Variable, VariableType, VariableValue, VectorNetwork,
};
use serde_json::Value;
use zip::ZipArchive;

use crate::{FormatError, add_image};

mod embedded_fonts;
mod export;
#[cfg(test)]
mod export_tests;
mod prototype;

pub use export::export;

const MAX_JSON: u64 = 16 * 1024 * 1024;
const MAX_JSON_VALUES: usize = 4_000_000;
const MAX_PAGE_SHAPES_BYTES: u64 = 128 * 1024 * 1024;
const MAX_IMAGE: u64 = 128 * 1024 * 1024;
const MAX_ARCHIVE: usize = 512 * 1024 * 1024;
const MAX_DECOMPRESSED: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
const MAX_LAYERS: usize = 50_000;
const MAX_TOTAL_IMAGES: usize = 512 * 1024 * 1024;
const MAX_DEPTH: usize = 128;
const ROOT_ID: &str = "00000000-0000-0000-0000-000000000000";

/// Report a malformed archive through the shared format error type.
fn corrupt(msg: impl Into<String>) -> FormatError {
    FormatError::Corrupt(format!("Penpot: {}", msg.into()))
}

/// Accept UUID-shaped identifiers before using them in archive member paths.
fn uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i, b)| if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() })
}

/// Counts actual ZIP output and JSON structure throughout one import, independently
/// of member header sizes and the serialized size of dense arrays/objects.
struct InflateBudget {
    remaining: u64,
    exceeded: bool,
    json_values: usize,
    json_exceeded: bool,
}

impl InflateBudget {
    fn new() -> Self {
        Self { remaining: MAX_DECOMPRESSED, exceeded: false, json_values: MAX_JSON_VALUES, json_exceeded: false }
    }

    fn check(&self) -> Result<(), FormatError> {
        if self.exceeded {
            Err(corrupt("actual decompressed ZIP contents exceed 1 GiB"))
        } else if self.json_exceeded {
            Err(corrupt("JSON structure exceeds import value budget"))
        } else {
            Ok(())
        }
    }

    /// Admit JSON before serde allocates its tree. Outside strings, each container,
    /// comma and colon covers at least one value or object key; counting one root
    /// as well deliberately overestimates structure. Serde still validates syntax.
    fn parse_json(&mut self, data: &[u8], path: &str) -> Result<Value, FormatError> {
        self.check()?;
        let mut values = 1;
        let (mut quoted, mut escaped) = (false, false);
        for &byte in data {
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
            } else if byte == b'"' {
                quoted = true;
            } else if matches!(byte, b'[' | b'{' | b',' | b':') {
                values += 1;
            }
        }
        if values > self.json_values {
            self.json_exceeded = true;
            return Err(corrupt("JSON structure exceeds import value budget"));
        }
        self.json_values -= values;
        serde_json::from_slice(data).map_err(|e| corrupt(format!("{path}: {e}")))
    }
}

struct BudgetReader<'a, R> {
    inner: R,
    budget: &'a mut InflateBudget,
}

impl<R: Read> Read for BudgetReader<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.budget.remaining == 0 {
            // Distinguish exact-budget EOF from data past the limit.
            let mut probe = [0_u8; 1];
            return match self.inner.read(&mut probe)? {
                0 => Ok(0),
                _ => {
                    self.budget.exceeded = true;
                    Err(io::Error::other("Penpot decompression budget exhausted"))
                }
            };
        }
        let limit = out.len().min(usize::try_from(self.budget.remaining).unwrap_or(usize::MAX));
        let read = self.inner.read(&mut out[..limit])?;
        self.budget.remaining -= read as u64;
        Ok(read)
    }
}

/// Read a JSON archive member within the per-member decompression limit.
fn json(z: &mut ZipArchive<Cursor<&[u8]>>, path: &str, budget: &mut InflateBudget) -> Result<Value, FormatError> {
    let entry = z.by_name(path).map_err(|e| corrupt(format!("{path}: {e}")))?;
    if entry.size() > MAX_JSON {
        return Err(corrupt(format!("JSON member too large: {path}")));
    }
    let mut data = Vec::new();
    BudgetReader { inner: entry, budget }.take(MAX_JSON + 1).read_to_end(&mut data).map_err(|e| corrupt(e.to_string()))?;
    if data.len() as u64 > MAX_JSON {
        return Err(corrupt(format!("JSON member too large: {path}")));
    }
    budget.parse_json(&data, path)
}

/// Read a shape JSON member within member and per-page decompression limits.
fn shape_json(z: &mut ZipArchive<Cursor<&[u8]>>, path: &str, budget: &mut InflateBudget, page_budget: &mut u64) -> Result<Value, FormatError> {
    let entry = z.by_name(path).map_err(|e| corrupt(format!("{path}: {e}")))?;
    if entry.size() > MAX_JSON {
        return Err(corrupt(format!("JSON member too large: {path}")));
    }
    if entry.size() > *page_budget {
        return Err(corrupt("page shapes exceed byte budget"));
    }
    let mut data = Vec::new();
    let max_read = (MAX_JSON + 1).min(*page_budget + 1);
    BudgetReader { inner: entry, budget }.take(max_read).read_to_end(&mut data).map_err(|e| corrupt(e.to_string()))?;
    if data.len() as u64 > MAX_JSON {
        return Err(corrupt(format!("JSON member too large: {path}")));
    }
    if data.len() as u64 > *page_budget {
        return Err(corrupt("page shapes exceed byte budget"));
    }
    *page_budget -= data.len() as u64;
    budget.parse_json(&data, path)
}

/// Read a finite numeric field or use the supplied fallback.
fn number(v: &Value, key: &str, default: f64) -> f64 {
    let n = v.get(key).and_then(|n| n.as_f64().or_else(|| n.as_str().and_then(|s| s.parse().ok()))).unwrap_or(default);
    if n.is_finite() { n.clamp(-1_000_000.0, 1_000_000.0) } else { default }
}
/// Read geometry from the shape or its saved selection rectangle.
fn geometry(v: &Value, key: &str, default: f64) -> f64 {
    match v.get(key) {
        Some(n) if !n.is_null() => number(v, key, default),
        _ => v.get("selrect").map_or(default, |s| number(s, key, default)),
    }
}
/// Borrow a string field without coercing other JSON types.
fn string<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}
/// Iterate an array field, treating absent or malformed arrays as empty.
fn list<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v.get(key).and_then(Value::as_array).into_iter().flatten()
}

/// Penpot ruler guides can be keyed objects or arrays. An `x` guide is a vertical
/// line; the coordinate is in the containing canvas/frame coordinate system.
/// Unknown axes and malformed positions are not imported as misleading guides.
fn penpot_guides(value: Option<&Value>) -> Vec<Guide> {
    let Some(value) = value else { return Vec::new() };
    let items: Box<dyn Iterator<Item = &Value> + '_> = match value {
        Value::Array(items) => Box::new(items.iter()),
        Value::Object(items) => Box::new(items.values()),
        _ => return Vec::new(),
    };
    items
        .take(500)
        .filter_map(|item| {
            let vertical = match string(item, "axis").or_else(|| string(item, "orientation"))? {
                "x" | "vertical" => true,
                "y" | "horizontal" => false,
                _ => return None,
            };
            let raw = item.get("position").or_else(|| item.get("offset"))?;
            let offset = raw.as_f64().or_else(|| raw.as_str().and_then(|s| s.parse::<f64>().ok()))?;
            (offset.is_finite() && offset.abs() <= 1_000_000.0).then_some(Guide { vertical, offset })
        })
        .collect()
}

/// Non-layout display grids shown on Penpot boards. These are separate from the
/// Flex/Grid auto-layout system. Unknown grids remain in source metadata.
fn display_grids(v: &Value, warnings: &mut Vec<String>) -> Vec<LayoutGrid> {
    let mut result = Vec::new();
    for grid in list(v, "grids").take(64) {
        let params = grid.get("params").unwrap_or(grid);
        let kind = match string(grid, "type") {
            Some("square") => GridKind::Grid,
            Some("column" | "columns") => GridKind::Columns,
            Some("row" | "rows") => GridKind::Rows,
            _ => {
                warn(warnings, "Unsupported Penpot display-grid type; original grid retained as metadata".into());
                continue;
            }
        };
        let mut native = match kind {
            GridKind::Grid => LayoutGrid::default(),
            GridKind::Columns => LayoutGrid::columns(),
            GridKind::Rows => LayoutGrid::rows(),
        };
        native.kind = kind;
        native.visible = grid.get("display").and_then(Value::as_bool) != Some(false);
        if kind == GridKind::Grid {
            native.size = number(params, "size", native.size).clamp(0.0, 100_000.0);
        } else {
            native.size = number(params, "itemLength", native.size).clamp(0.0, 100_000.0);
            native.count = params
                .get("size")
                .or_else(|| params.get("count"))
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && *n >= 0.0)
                .map_or(native.count, |n| n.min(10_000.0) as u32);
        }
        native.gutter = number(params, "gutter", native.gutter).clamp(0.0, 100_000.0);
        native.offset = number(params, "margin", native.offset);
        native.align = match string(params, "type").or_else(|| string(params, "align")) {
            Some("left" | "top" | "start") => GridAlign::Min,
            Some("center") => GridAlign::Center,
            Some("right" | "bottom" | "end") => GridAlign::Max,
            _ => GridAlign::Stretch,
        };
        if let Some(c) = params.get("color").or_else(|| grid.get("color"))
            && let Some(hex) = c.as_str().or_else(|| string(c, "color"))
            && let Some(color) = Color::from_hex(hex)
        {
            native.color = color.with_alpha((color.a as f64 * number(c, "opacity", 1.0).clamp(0.0, 1.0)) as f32);
        }
        result.push(native);
    }
    result
}

/// Resolve grid placement and track sizing after the children have been imported.
fn finish_layout(doc: &mut Document, frame: NodeId, v: &Value, warnings: &mut Vec<String>) {
    let Some(f) = doc.node(frame) else { return };
    let Some(al) = f.layout.clone() else { return };
    let kids: Vec<figcraft_doc::Node> = f.children.iter().filter_map(|id| doc.node(*id)).filter(|n| !n.child_layout.absolute).cloned().collect();
    if kids.iter().any(|n| n.plugin_data.contains_key("penpot.unsupported.itemLayout")) {
        if let Some(n) = doc.node_mut(frame) {
            n.layout = None;
        }
        warn(warnings, "Penpot item margins or self-alignment are unsupported; saved positions and source metadata were kept".into());
        return;
    }
    let (width, height, hug_w, hug_h) = (f.width, f.height, f.child_layout.sizing_h == Sizing::Hug, f.child_layout.sizing_v == Sizing::Hug);
    let [pt, pr, pb, pl] = al.padding;
    let mut al = al;
    match al.mode {
        LayoutMode::Horizontal | LayoutMode::Vertical => {
            let horizontal = al.mode == LayoutMode::Horizontal;
            let grows = kids.iter().any(|n| if horizontal { n.child_layout.sizing_h } else { n.child_layout.sizing_v } == Sizing::Fill);
            if grows && matches!(al.main_align, MainAlign::SpaceBetween | MainAlign::SpaceAround | MainAlign::SpaceEvenly) {
                al.main_align = MainAlign::Min;
            }
        }
        LayoutMode::Grid => {
            if string(v, "layoutJustifyContent") == Some("stretch") && !hug_w {
                let content = kids
                    .iter()
                    .filter(|n| n.child_layout.column_span() == 1 && n.child_layout.sizing_h != Sizing::Fill)
                    .filter_map(|n| Some((n.child_layout.grid_column?, n.width)));
                al.column_tracks = stretch_auto_tracks(&al.column_tracks, content, width - pl - pr, al.gap);
            }
            if string(v, "layoutAlignContent") == Some("stretch") && !hug_h {
                let content = kids
                    .iter()
                    .filter(|n| n.child_layout.row_span() == 1 && n.child_layout.sizing_v != Sizing::Fill)
                    .filter_map(|n| Some((n.child_layout.grid_row?, n.height)));
                al.row_tracks = stretch_auto_tracks(&al.row_tracks, content, height - pt - pb, al.cross_gap);
            }
        }
    }
    if let Some(n) = doc.node_mut(frame) {
        n.layout = Some(al);
    }
}

/// Convert auto tracks to proportional fill tracks using their measured content.
fn stretch_auto_tracks(tracks: &[GridTrack], content: impl Iterator<Item = (u32, f64)>, inner: f64, gap: f64) -> Vec<GridTrack> {
    if tracks.iter().any(|t| t.sizing == Sizing::Fill) {
        return tracks.to_vec();
    }
    let mut sizes: Vec<f64> = tracks.iter().map(|t| if t.sizing == Sizing::Fixed { t.value } else { 0.0 }).collect();
    for (i, size) in content {
        if let (Some(s), Some(GridTrack { sizing: Sizing::Hug, .. })) = (sizes.get_mut(i as usize), tracks.get(i as usize)) {
            *s = s.max(size);
        }
    }
    let autos = tracks.iter().filter(|t| t.sizing == Sizing::Hug).count();
    let free = inner - sizes.iter().sum::<f64>() - gap * tracks.len().saturating_sub(1) as f64;
    if autos == 0 || !free.is_finite() || free <= 0.0 {
        return tracks.to_vec();
    }
    let share = free / autos as f64;
    tracks.iter().zip(&sizes).map(|(t, s)| if t.sizing == Sizing::Hug { GridTrack::fill(s + share) } else { *t }).collect()
}

/// Convert a supported group mask and report masks that cannot be represented.
fn finish_masked_group(doc: &mut Document, group: NodeId, source: &Value, warnings: &mut Vec<String>) {
    if source.get("maskedGroup").and_then(Value::as_bool) != Some(true) {
        return;
    }
    let Some(children) = doc.node(group).map(|n| n.children.clone()) else {
        return;
    };
    let first = list(source, "shapes").filter_map(Value::as_str).next();
    let mask = first
        .and_then(|first| children.iter().copied().find(|&id| doc.node(id).and_then(|n| n.plugin_data.get("penpot.id")).is_some_and(|p| p == first)));
    let Some(mask) = mask else {
        warn(warnings, "Penpot masked group has no first layer to use as its mask; original group preserved".into());
        return;
    };
    for &id in &children {
        if let Some(child) = doc.node_mut(id) {
            child.is_mask = id == mask;
        }
    }
    if let Some(node) = doc.node_mut(group) {
        node.children.retain(|id| *id != mask);
        node.children.insert(0, mask);
        node.plugin_data.insert("penpot.maskImported".into(), "true".into());
    }
}

/// Read one exported `.penpot` archive without touching the filesystem or running a converter.
/// Returns native, editable FigCraft nodes plus notes about unsupported features.
pub fn import(bytes: &[u8]) -> Result<(Document, Vec<String>), FormatError> {
    if bytes.len() > MAX_ARCHIVE {
        return Err(corrupt("archive exceeds 512 MiB"));
    }
    let mut z = ZipArchive::new(Cursor::new(bytes)).map_err(|e| corrupt(format!("invalid ZIP: {e}")))?;
    if z.len() > MAX_ENTRIES {
        return Err(corrupt("too many ZIP members"));
    }
    // Reject oversized logical archives before decoding any JSON. This guards against ZIP bombs
    // whose compressed size is innocuous but whose advertised uncompressed size is enormous.
    let mut expanded = 0_u64;
    let mut names = BTreeSet::new();
    for index in 0..z.len() {
        let member = z.by_index(index).map_err(|e| corrupt(e.to_string()))?;
        // An importer never extracts members to disk, but ambiguous ZIP names can
        // cause one member to be inspected and another member to be read by name.
        // Reject duplicates and traversal instead of guessing which version is real.
        let name = member.name();
        if !names.insert(name.to_owned()) {
            return Err(corrupt(format!("duplicate ZIP member: {}", name.chars().take(200).collect::<String>())));
        }
        if name.starts_with('/')
            || name.contains('\\')
            || name.split('/').any(|part| part == "." || part == "..")
            || name.chars().any(char::is_control)
        {
            return Err(corrupt("unsafe ZIP member path"));
        }
        expanded = expanded.saturating_add(member.size());
        if expanded > MAX_DECOMPRESSED {
            return Err(corrupt("ZIP contents exceed 1 GiB"));
        }
    }
    let mut inflate_budget = InflateBudget::new();
    let manifest = json(&mut z, "manifest.json", &mut inflate_budget)?;
    if string(&manifest, "type") != Some("penpot/export-files") {
        return Err(corrupt("not a Penpot export-files archive"));
    }
    if manifest.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(corrupt("unsupported Penpot export version"));
    }
    let files = manifest.get("files").and_then(Value::as_array).ok_or_else(|| corrupt("no files in manifest"))?;
    if files.is_empty() || files.len() > 50 {
        return Err(corrupt("invalid number of Penpot files"));
    }

    let initial_name = files.first().and_then(|f| string(f, "name")).unwrap_or("Penpot import");
    let mut doc = Document::without_pages(initial_name.chars().take(256).collect::<String>());
    let mut warnings = Vec::new();
    embedded_fonts::report(&mut z, &mut warnings, &mut inflate_budget);
    inflate_budget.check()?;
    let mut image_keys: BTreeMap<String, String> = BTreeMap::new();
    let mut layer_count = 0;
    let mut image_bytes = 0usize;
    let mut svg_bytes = 0usize;
    let mut token_sources = Vec::new();
    let mut active_modes = BTreeMap::new();

    for file in files {
        let Some(file_id) = string(file, "id").filter(|id| uuid(id)) else {
            return Err(corrupt("invalid file UUID"));
        };
        let base = format!("files/{file_id}");
        let (masters, variant_names, component_sources) = component_masters(&mut z, &base, &mut inflate_budget);
        let file_header = json(&mut z, &format!("{base}.json"), &mut inflate_budget)
            .ok()
            .map(|header| {
                let kept: serde_json::Map<String, Value> =
                    FILE_HEADER_KEYS.iter().filter_map(|key| Some(((*key).to_owned(), header.get(*key)?.clone()))).collect();
                Value::Object(kept)
            })
            .and_then(|kept| serde_json::to_string(&kept).ok())
            .filter(|encoded| encoded.len() <= 16_384);
        // Library swatches remain useful as native document colors even when a
        // Penpot shared-style/variable reference has no FigCraft equivalent.
        import_document_colors(&mut z, &base, &mut doc, &mut warnings, &mut inflate_budget);
        let library_font_refs = import_typography_styles(&mut z, &base, &mut doc, &mut warnings, &mut inflate_budget);
        if let Some(source) = import_token_variables(&mut z, &base, &mut doc, &mut active_modes, &mut warnings, &mut inflate_budget) {
            token_sources.push((file_id.to_owned(), source));
        }
        inflate_budget.check()?;
        // The page index files are peers of the directories containing their layers.
        let prefix = format!("{base}/pages/");
        let page_names: Vec<String> = z
            .file_names()
            .filter(|name| {
                name.strip_prefix(&prefix).is_some_and(|part| part.len() == 41 && part.ends_with(".json") && part.get(..36).is_some_and(uuid))
            })
            .take(501)
            .map(str::to_owned)
            .collect();
        if page_names.len() > 500 {
            return Err(corrupt("too many pages"));
        }
        let mut pages = Vec::new();
        for path in page_names {
            let value = json(&mut z, &path, &mut inflate_budget)?;
            let Some(id) = string(&value, "id").filter(|id| uuid(id)) else { continue };
            let name = string(&value, "name").unwrap_or("Page").chars().take(256).collect::<String>();
            let index = value.get("index").and_then(Value::as_i64).unwrap_or(i64::MAX);
            let bg = string(&value, "background").and_then(Color::from_hex);
            let guides = penpot_guides(value.get("guides").or_else(|| value.pointer("/options/guides")));
            // The page's flows name the frames the prototype starts at; they are read once
            // every page is in the document, when the frame ids can be resolved.
            let flows = value.get("flows").cloned();
            pages.push((index, id.to_owned(), name, bg, guides, flows));
        }
        pages.sort_by_key(|p| p.0);
        let mut library_refs_attached = false;
        let mut file_header = file_header;
        for (_, page_id, page_name, bg, guides, flows) in pages {
            let page = doc.add_page(&page_name);
            if let Some(node) = doc.node_mut(page) {
                node.plugin_data.insert("penpot.pageId".into(), page_id.clone());
                node.plugin_data.insert("penpot.fileId".into(), file_id.to_owned());
                if let Some(header) = file_header.take() {
                    node.plugin_data.insert("penpot.source.file".into(), header);
                }
            }
            if !library_refs_attached && !library_font_refs.is_empty() {
                if let Some(node) = doc.node_mut(page) {
                    node.plugin_data.insert("penpot.customLibraryFontRefs".into(), serde_json::to_string(&library_font_refs).unwrap_or_default());
                }
                library_refs_attached = true;
            }
            if let Some(node) = doc.node_mut(page) {
                node.guides = guides;
            }
            if let Some(bg) = bg
                && let Some(node) = doc.node_mut(page)
                && let NodeKind::Page { background, .. } = &mut node.kind
            {
                *background = bg;
            }
            // The page's flows name the frames the prototype starts at. They are read once
            // every page is in the document, when the frame ids can be resolved.
            if let Some(flows) = flows
                && let Ok(encoded) = serde_json::to_string(&flows)
                && encoded.len() <= 16_384
                && let Some(node) = doc.node_mut(page)
            {
                node.plugin_data.insert("penpot.flows".into(), encoded);
            }
            let page_prefix = format!("{prefix}{page_id}/");
            let paths: Vec<String> = z
                .file_names()
                .filter(|name| {
                    name.strip_prefix(&page_prefix)
                        .is_some_and(|part| part.len() == 41 && part.ends_with(".json") && part.get(..36).is_some_and(uuid))
                })
                .take(MAX_LAYERS + 1)
                .map(str::to_owned)
                .collect();
            if paths.len() > MAX_LAYERS {
                return Err(corrupt("too many layers on a page"));
            }
            let mut shapes = BTreeMap::new();
            let mut page_shape_budget = MAX_PAGE_SHAPES_BYTES;
            for path in paths {
                let shape = shape_json(&mut z, &path, &mut inflate_budget, &mut page_shape_budget)?;
                if let Some(id) = string(&shape, "id").filter(|id| uuid(id)) {
                    shapes.insert(id.to_owned(), shape);
                }
            }
            let Some(root) = shapes.get(ROOT_ID) else {
                return Err(corrupt(format!("page {page_name} has no root frame")));
            };
            // The root frame represents the page canvas rather than a visible 0.01 px frame.
            if let Some(c) = list(root, "fills").find_map(|f| string(f, "fillColor").and_then(Color::from_hex))
                && let Some(node) = doc.node_mut(page)
                && let NodeKind::Page { background, .. } = &mut node.kind
            {
                *background = c;
            }
            // The reverse parent index lets us recover children omitted from `shapes`
            // without a quadratic scan for every nested group (large files have many layers).
            // Declared stacking order always wins; recovered siblings are deterministic.
            let mut children_by_parent: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for (shape_id, shape) in &shapes {
                if shape_id != ROOT_ID
                    && let Some(pid) = string(shape, "parentId").filter(|id| uuid(id))
                {
                    children_by_parent.entry(pid.to_owned()).or_default().push(shape_id.clone());
                }
            }
            let svg_defs = svg_gradients(&shapes);
            let roots: Vec<String> = list(root, "shapes").filter_map(Value::as_str).map(str::to_owned).collect();
            let mut seen = BTreeSet::new();
            let mut layers = LayerImport {
                z: &mut z,
                base: &base,
                shapes: &shapes,
                children_by_parent: &children_by_parent,
                svg_defs: &svg_defs,
                masters: &masters,
                doc: &mut doc,
                images: &mut image_keys,
                image_bytes: &mut image_bytes,
                inflate_budget: &mut inflate_budget,
                svg_bytes: &mut svg_bytes,
                warnings: &mut warnings,
                seen: &mut seen,
                count: &mut layer_count,
            };
            for id in roots {
                layers.add_layer(&id, page, Affine::IDENTITY, None, 0)?;
            }
            // Recover root children not listed in the export's stacking-order array.
            if let Some(unlinked) = children_by_parent.get(ROOT_ID) {
                for id in unlinked {
                    if !layers.seen.contains(id) {
                        warn(layers.warnings, "Penpot page omitted a top-level child from its stacking list; recovered it by parentId".into());
                        layers.add_layer(id, page, Affine::IDENTITY, None, 0)?;
                    }
                }
            }
            // Broken exports can contain valid shape JSON with a missing, invalid,
            // or cyclic parent chain. Earlier versions silently omitted those
            // objects because walking from the page root never reached them.
            // First recover the true orphan roots, preserving their descendants
            // with the original global Penpot coordinates.
            for (id, shape) in &shapes {
                if id == ROOT_ID || layers.seen.contains(id) {
                    continue;
                }
                let parent_id = string(shape, "parentId");
                let parent_exists = parent_id.is_some_and(|pid| pid != id.as_str() && shapes.contains_key(pid));
                if !parent_exists {
                    warn(
                        layers.warnings,
                        format!(
                            "Penpot orphan layer '{}' has no reachable parent; attached to page",
                            string(shape, "name").unwrap_or(id).chars().take(80).collect::<String>()
                        ),
                    );
                    layers.add_layer(id, page, Affine::IDENTITY, None, 0)?;
                }
            }
            // Remaining disconnected subgraphs can be caused by cycles or a
            // parent chain deeper than MAX_DEPTH. Import one deterministic root
            // for each instead of dropping editable content without warning.
            for id in shapes.keys() {
                if id != ROOT_ID && !layers.seen.contains(id) {
                    warn(layers.warnings, format!("Penpot disconnected/cyclic layer graph at {id}; recovered as a page layer"));
                    layers.add_layer(id, page, Affine::IDENTITY, None, 0)?;
                }
            }
        }
        inflate_budget.check()?;
        name_variants(&mut doc, file_id, &variant_names);
        keep_component_sources(&mut doc, file_id, &component_sources);
    }
    inflate_budget.check()?;
    if doc.pages.is_empty() {
        return Err(corrupt("no pages found"));
    }
    doc.current_page = doc.first_page();
    // Penpot's multidimensional themes and original token aliases cannot safely be
    // flattened to a single FigCraft mode. Store the bounded original on the first
    // page, which survives saving as .figcraft; never claim live shape bindings.
    if let Some(page) = doc.first_page()
        && let Some(node) = doc.node_mut(page)
    {
        for (file, source) in token_sources {
            node.plugin_data.insert(format!("penpot.tokens.{file}"), source);
        }
    }
    for page in doc.pages.clone() {
        if let Some(node) = doc.node_mut(page) {
            node.var_modes.extend(active_modes.clone());
        }
    }
    doc.repair();
    link_local_instances(&mut doc, &mut warnings);
    // Map safe, unambiguous Penpot applied-token references to native bindings
    // after the instance graph is built, without changing the imported paint/geometry.
    bind_applied_tokens(&mut doc, &active_modes, &mut warnings);
    // The prototype links name shapes by their Penpot ids, which only resolve once every
    // page is in the document.
    import_prototype_links(&mut doc, &mut warnings);
    let structural_issues = doc.validate();
    if !structural_issues.is_empty() {
        warn(
            &mut warnings,
            format!("FigCraft document has {} remaining structural issue(s) after Penpot import; review before editing", structural_issues.len()),
        );
    }
    report_unsupported_layout(&doc, &mut warnings);
    report_design_system_limits(&doc, &mut warnings);
    record_fingerprints(&mut doc);
    Ok((doc, warnings))
}

const FILE_HEADER_KEYS: &[&str] = &["features", "version", "migrations", "options", "isShared", "createdAt", "modifiedAt"];

/// Hash serialized native state so export can detect edits to imported properties.
pub(crate) fn fingerprint(value: &impl serde::Serialize) -> Option<String> {
    let bytes = serde_json::to_vec(value).ok()?;
    Some(crate::image_hash(&bytes).chars().take(32).collect())
}

/// Fingerprint a layer layout together with its child layout settings.
pub(crate) fn layout_fingerprint(doc: &Document, id: NodeId) -> Option<String> {
    let node = doc.node(id)?;
    let children: Vec<Value> =
        node.children.iter().filter_map(|c| doc.node(*c)).map(|c| serde_json::json!([c.plugin_data.get("penpot.id"), c.child_layout])).collect();
    fingerprint(&serde_json::json!({"layout": node.layout, "children": children}))
}

/// Fingerprint native text and its paints for source-preserving export.
pub(crate) fn text_fingerprint(node: &figcraft_doc::Node) -> Option<String> {
    match &node.kind {
        NodeKind::Text(text) => fingerprint(&serde_json::json!({"text": text, "fills": node.fills})),
        _ => None,
    }
}

/// Fingerprint stroke paints and stroke geometry settings.
pub(crate) fn strokes_fingerprint(node: &figcraft_doc::Node) -> Option<String> {
    fingerprint(&serde_json::json!({"strokes": node.strokes, "stroke": node.stroke}))
}

/// Fingerprint native variables for source-preserving token export.
pub(crate) fn tokens_fingerprint(doc: &Document) -> Option<String> {
    fingerprint(&doc.variables)
}

/// Store import-time fingerprints used to preserve unchanged Penpot source.
fn record_fingerprints(doc: &mut Document) {
    let mut found = Vec::new();
    for node in doc.nodes() {
        let layout = node.plugin_data.contains_key("penpot.source.layout").then(|| layout_fingerprint(doc, node.id)).flatten();
        let text = node.plugin_data.contains_key("penpot.source.content").then(|| text_fingerprint(node)).flatten();
        let interactions = node.plugin_data.contains_key("penpot.source.interactions").then(|| fingerprint(&node.interactions)).flatten();
        let strokes = node.plugin_data.contains_key("penpot.source.strokes").then(|| strokes_fingerprint(node)).flatten();
        if layout.is_some() || text.is_some() || interactions.is_some() || strokes.is_some() {
            found.push((node.id, layout, text, interactions, strokes));
        }
    }
    for (id, layout, text, interactions, strokes) in found {
        if let Some(node) = doc.node_mut(id) {
            for (key, value) in
                [("penpot.fp.layout", layout), ("penpot.fp.text", text), ("penpot.fp.interactions", interactions), ("penpot.fp.strokes", strokes)]
            {
                if let Some(value) = value {
                    node.plugin_data.insert(key.into(), value);
                }
            }
        }
    }
    let tokens = tokens_fingerprint(doc);
    if let Some(page) = doc.first_page()
        && let Some(tokens) = tokens
        && let Some(node) = doc.node_mut(page)
        && node.plugin_data.keys().any(|k| k.starts_with("penpot.tokens."))
    {
        node.plugin_data.insert("penpot.fp.tokens".into(), tokens);
    }
}

/// Retain original component JSON for exporting imported component definitions.
fn keep_component_sources(doc: &mut Document, file_id: &str, sources: &BTreeMap<String, String>) {
    let ids: Vec<(NodeId, String)> = doc
        .nodes()
        .filter(|n| matches!(n.kind, NodeKind::Component { .. }))
        .filter(|n| n.plugin_data.get("penpot.fileId").map(String::as_str) == Some(file_id))
        .filter_map(|n| Some((n.id, sources.get(n.plugin_data.get("penpot.id")?)?.clone())))
        .collect();
    for (id, source) in ids {
        if let Some(node) = doc.node_mut(id) {
            node.plugin_data.insert("penpot.source.component".into(), source);
        }
    }
}

/// Turn the imported Penpot prototype links into working FigCraft interactions and flows.
///
/// Every shape keeps its Penpot interactions in its metadata, and every page its Penpot
/// flows. Now that the whole document is in, the Penpot shape ids they name can be
/// resolved to FigCraft node ids.
fn import_prototype_links(doc: &mut Document, warnings: &mut Vec<String>) {
    let by_penpot_id: BTreeMap<String, NodeId> = doc.nodes().filter_map(|n| Some((n.plugin_data.get("penpot.id")?.clone(), n.id))).collect();
    prototype::resolve_destinations(doc, &by_penpot_id, warnings);
    // The pages' flows name the frames the prototype starts at.
    let pages: Vec<NodeId> = doc.pages.clone();
    for page in pages {
        let Some(raw) = doc.node(page).and_then(|n| n.plugin_data.get("penpot.flows")).cloned() else { continue };
        let Ok(flows) = serde_json::from_str::<Value>(&raw) else { continue };
        let read = prototype::page_flows(doc, &flows, &by_penpot_id, warnings);
        if read.is_empty() {
            continue;
        }
        if let Some(node) = doc.node_mut(page)
            && let NodeKind::Page { flows: page_flows, .. } = &mut node.kind
        {
            page_flows.extend(read);
        }
    }
}

/// Collect readable, bounded layout compatibility warnings after importing every page.
/// Per-layer source properties survive a save/open cycle in `plugin_data`, even where
/// FigCraft's current layout engine does not expose the same feature.
fn report_unsupported_layout(doc: &Document, warnings: &mut Vec<String>) {
    for (flag, explanation) in [("penpot.unsupported.alignSelf", "unrecognized per-element alignment")] {
        let mut count = 0usize;
        let mut examples = Vec::new();
        for node in doc.nodes() {
            if node.plugin_data.contains_key(flag) {
                count += 1;
                if examples.len() < 3 {
                    examples.push(format!("'{}'", node.name.chars().filter(|c| !c.is_control()).take(80).collect::<String>()));
                }
            }
        }
        if count > 0 {
            warn(
                warnings,
                format!(
                    "Penpot: {count} layer(s) use unsupported {explanation}; original values are preserved in layer metadata, but may change visually after auto-layout edits. Example(s): {}",
                    examples.join(", "),
                ),
            );
        }
    }
}

/// Link fully mapped local Penpot component copies to FigCraft's native component graph.
///
/// A Penpot instance may have detached, reordered or injected layers. Syncing one of those as a
/// FigCraft instance would *delete content*. We therefore link only exact shapeRef trees, and
/// leave every other copy independent with its original metadata intact.
fn link_local_instances(doc: &mut Document, warnings: &mut Vec<String>) {
    let mut masters = BTreeMap::new();
    for node in doc.nodes() {
        if !matches!(node.kind, NodeKind::Component { .. }) {
            continue;
        }
        if let (Some(file), Some(component)) = (node.plugin_data.get("penpot.fileId"), node.plugin_data.get("penpot.componentMasterId")) {
            masters.insert((file.clone(), component.clone()), node.id);
        }
    }
    let by_penpot_id: BTreeMap<String, NodeId> = doc.nodes().filter_map(|n| Some((n.plugin_data.get("penpot.id")?.clone(), n.id))).collect();
    let mut candidates: Vec<(bool, usize, NodeId)> = doc
        .nodes()
        .filter(|n| n.plugin_data.contains_key("penpot.componentLinkStatus"))
        .map(|n| {
            let ancestors = doc.ancestors(n.id);
            let in_main = ancestors.iter().any(|a| doc.node(*a).is_some_and(|a| matches!(a.kind, NodeKind::Component { .. })));
            (!in_main, if in_main { usize::MAX - ancestors.len() } else { ancestors.len() }, n.id)
        })
        .collect();
    candidates.sort();
    let candidates: Vec<NodeId> = candidates.into_iter().map(|(_, _, id)| id).collect();
    let mut missing_masters = false;
    let mut editor_state_from_main = 0;
    for id in candidates {
        let Some(n) = doc.node(id) else { continue };
        // Copies nested in another copy are dealt with separately. Do not turn their root into
        // an instance until the surrounding master/instance graph can be safely synchronized.
        if doc.ancestors(id).iter().any(|a| doc.node(*a).is_some_and(|n| matches!(n.kind, NodeKind::Instance { .. }))) {
            continue;
        }
        let Some(file) = n.plugin_data.get("penpot.componentFile").or_else(|| n.plugin_data.get("penpot.fileId")) else { continue };
        let Some(component_id) = n.plugin_data.get("penpot.componentId") else { continue };
        let Some(&main) = masters.get(&(file.clone(), component_id.clone())) else {
            missing_masters = true;
            continue;
        };
        if doc.is_ancestor(main, id) || doc.is_ancestor(id, main) {
            continue;
        }
        let mut pairs = Vec::new();
        if let Err(reason) = match_instance_tree(doc, &by_penpot_id, id, main, true, &mut pairs, 0) {
            warn(warnings, format!("Component copy '{}' kept independent: {reason}", n.name));
            if let Some(node) = doc.node_mut(id) {
                node.plugin_data.insert("penpot.componentLinkReason".into(), reason);
            }
            continue;
        }
        editor_state_from_main += pairs
            .iter()
            .filter(|(copy, source, _)| {
                *copy != id
                    && doc
                        .node(*copy)
                        .zip(doc.node(*source))
                        .is_some_and(|(c, s)| c.locked != s.locked || c.constrain_proportions != s.constrain_proportions)
            })
            .count();
        let nested_kind: BTreeMap<NodeId, NodeKind> = pairs
            .iter()
            .filter(|(copy, _, _)| *copy != id)
            .filter_map(|(copy, source, _)| match doc.node(*source)?.kind.clone() {
                kind @ NodeKind::Instance { .. } => Some((*copy, kind)),
                _ => None,
            })
            .collect();
        for (copy, source, overrides) in pairs {
            if let Some(node) = doc.node_mut(copy) {
                node.source = Some(source);
                node.overrides = overrides;
                if copy == id {
                    node.kind = NodeKind::Instance { component: main, props: BTreeMap::new() };
                    node.plugin_data.insert("penpot.componentLinkStatus".into(), "linked".into());
                } else if let Some(kind) = nested_kind.get(&copy) {
                    node.kind = kind.clone();
                    node.plugin_data.insert("penpot.componentLinkStatus".into(), "linked".into());
                } else if node.plugin_data.contains_key("penpot.componentLinkStatus") {
                    node.plugin_data.insert("penpot.componentLinkStatus".into(), "layer-of-linked-instance".into());
                }
            }
        }
    }
    if missing_masters {
        warn(warnings, "Some Penpot component copies could not find their local masters; kept as editable independent frames".into());
    }
    if editor_state_from_main > 0 {
        warn(
            warnings,
            format!(
                "{editor_state_from_main} layer(s) inside linked component copies now take their lock and proportion-lock state from the main component"
            ),
        );
    }
}

/// Preflight an instance and its descendants. The order of children must match exactly because
/// FigCraft's synchronization follows the master's order. Mapping by name alone is unsafe.
fn refers_to(doc: &Document, by_penpot_id: &BTreeMap<String, NodeId>, copy: &figcraft_doc::Node, source: &figcraft_doc::Node) -> bool {
    let Some(target) = source.plugin_data.get("penpot.id") else { return false };
    let mut reference = copy.plugin_data.get("penpot.shapeRef");
    for _ in 0..MAX_DEPTH {
        let Some(r) = reference else { return false };
        if r == target {
            return true;
        }
        reference = by_penpot_id.get(r).and_then(|id| doc.node(*id)).and_then(|n| n.plugin_data.get("penpot.shapeRef"));
    }
    false
}

/// Compare JSON values while tolerating small numeric differences.
fn nearly_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() <= 0.01,
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| nearly_equal(x, y)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| nearly_equal(v, w))),
        _ => a == b,
    }
}

/// Check whether two layer kinds have compatible geometric data.
fn same_geometry(a: &NodeKind, b: &NodeKind) -> bool {
    match (serde_json::to_value(a), serde_json::to_value(b)) {
        (Ok(a), Ok(b)) => nearly_equal(&a, &b),
        _ => false,
    }
}

/// Validate corresponding instance descendants before committing their links.
fn match_instance_tree(
    doc: &Document,
    by_penpot_id: &BTreeMap<String, NodeId>,
    copy_id: NodeId,
    source_id: NodeId,
    is_root: bool,
    pairs: &mut Vec<(NodeId, NodeId, BTreeSet<String>)>,
    depth: usize,
) -> Result<(), String> {
    if depth >= MAX_DEPTH {
        return Err("nesting too deep".into());
    }
    let (Some(copy), Some(source)) = (doc.node(copy_id), doc.node(source_id)) else { return Err("missing layer".into()) };
    let at = |what: &str| format!("{what} at '{}'", copy.name.chars().take(60).collect::<String>());
    if !refers_to(doc, by_penpot_id, copy, source) {
        return Err(at("layer does not reference its main counterpart"));
    }
    let kinds_match = if is_root {
        matches!((&copy.kind, &source.kind), (NodeKind::Frame, NodeKind::Component { .. }))
    } else {
        // Paths, booleans and component sets must be byte-for-byte equivalent: their geometry
        // cannot be carried by FigCraft's generic instance override groups.
        match (&copy.kind, &source.kind) {
            (NodeKind::Text(_), NodeKind::Text(_)) => true,
            (NodeKind::Frame, NodeKind::Instance { .. }) => true,
            (a, b) => same_geometry(a, b) && !matches!(a, NodeKind::Component { .. } | NodeKind::Instance { .. } | NodeKind::ComponentSet { .. }),
        }
    };
    if !kinds_match {
        return Err(at("layer type differs from main"));
    }
    if copy.children.len() != source.children.len() {
        return Err(at("children added or removed"));
    }
    // Nested Penpot component instances have their own source/override graph.
    if !is_root
        && matches!(source.kind, NodeKind::Instance { .. })
        && copy.plugin_data.get("penpot.componentId") != source.plugin_data.get("penpot.componentId")
    {
        return Err(at("swapped nested component instance"));
    }
    // These fields have no reliable component override in FigCraft yet. Any difference must
    // keep the copy independent so a later component sync cannot silently erase it.
    let blocks = |differs: bool| differs && !is_root;
    if copy.clip != source.clip
        || blocks(copy.is_mask != source.is_mask)
        || copy.layout != source.layout
        || blocks(copy.constraints != source.constraints)
        || blocks(copy.child_layout.absolute != source.child_layout.absolute)
        || copy.grids != source.grids
        || copy.overflow != source.overflow
        || copy.mask_type != source.mask_type
        || blocks(copy.guides != source.guides)
        || blocks(copy.exports != source.exports)
        || blocks(copy.interactions != source.interactions)
        || copy.styles != source.styles
        || copy.bound != source.bound
        || blocks(copy.var_modes != source.var_modes)
    {
        return Err(at(&format!("unsupported override ({})", differing_fields(copy, source).join(", "))));
    }
    let mut overrides = BTreeSet::new();
    if copy.name != source.name {
        overrides.insert("name".into());
    }
    if copy.fills != source.fills {
        overrides.insert("fills".into());
    }
    if copy.strokes != source.strokes || copy.stroke != source.stroke {
        overrides.insert("strokes".into());
    }
    if copy.effects != source.effects {
        overrides.insert("effects".into());
    }
    if copy.opacity != source.opacity || copy.blend != source.blend {
        overrides.insert("opacity".into());
    }
    if copy.visible != source.visible {
        overrides.insert("visible".into());
    }
    if copy.corners != source.corners {
        overrides.insert("corners".into());
    }
    if copy.width != source.width || copy.height != source.height || copy.child_layout != source.child_layout {
        overrides.insert("size".into());
    }
    if !is_root && copy.transform != source.transform {
        overrides.insert("position".into());
    }
    if let (NodeKind::Text(a), NodeKind::Text(b)) = (&copy.kind, &source.kind) {
        if a.characters != b.characters || a.runs != b.runs || a.paragraphs != b.paragraphs {
            overrides.insert("characters".into());
        }
        if a.style != b.style
            || a.auto_resize != b.auto_resize
            || a.align_h != b.align_h
            || a.align_v != b.align_v
            || a.paragraph_spacing != b.paragraph_spacing
            || a.paragraph_indent != b.paragraph_indent
            || a.list != b.list
            || a.list_spacing != b.list_spacing
            || a.hanging_punctuation != b.hanging_punctuation
            || a.hanging_list != b.hanging_list
            || a.max_lines != b.max_lines
            || a.features != b.features
            || a.leading_trim != b.leading_trim
            || a.path != b.path
        {
            overrides.insert("textStyle".into());
        }
    }
    pairs.push((copy_id, source_id, overrides));
    for (&child, &orig) in copy.children.iter().zip(&source.children) {
        match_instance_tree(doc, by_penpot_id, child, orig, false, pairs, depth + 1)?;
    }
    Ok(())
}

/// Identify native property groups overridden by an imported instance.
fn differing_fields(copy: &figcraft_doc::Node, source: &figcraft_doc::Node) -> Vec<&'static str> {
    [
        ("clip", copy.clip != source.clip),
        ("mask", copy.is_mask != source.is_mask),
        ("layout", copy.layout != source.layout),
        ("constraints", copy.constraints != source.constraints),
        ("absolute", copy.child_layout.absolute != source.child_layout.absolute),
        ("grids", copy.grids != source.grids),
        ("overflow", copy.overflow != source.overflow),
        ("mask type", copy.mask_type != source.mask_type),
        ("guides", copy.guides != source.guides),
        ("exports", copy.exports != source.exports),
        ("interactions", copy.interactions != source.interactions),
        ("styles", copy.styles != source.styles),
        ("variable bindings", copy.bound != source.bound),
        ("variable modes", copy.var_modes != source.var_modes),
    ]
    .into_iter()
    .filter(|(_, differs)| *differs)
    .map(|(name, _)| name)
    .collect()
}

/// Return a compact, inspectable snapshot of unsupported Penpot geometry/properties. Keeping
/// the source data makes later import upgrades and manual reconstruction possible after saving.
fn preserve_source(node: &mut figcraft_doc::Node, v: &Value, keys: &[&str]) {
    preserve_source_within(node, v, keys, 16_384);
}

/// Retain selected source fields without exceeding the metadata size budget.
fn preserve_source_within(node: &mut figcraft_doc::Node, v: &Value, keys: &[&str], limit: usize) {
    for &key in keys {
        if let Some(value) = v.get(key) {
            // Never fill a document with arbitrary megabytes of opaque source JSON.
            if let Ok(encoded) = serde_json::to_string(value)
                && encoded.len() <= limit
            {
                node.plugin_data.insert(format!("penpot.source.{key}"), encoded);
            }
        }
    }
}

/// Translate Penpot sizing names to fixed, fill or hug sizing.
fn penpot_sizing(value: Option<&str>) -> Sizing {
    match value {
        Some("auto" | "hug") => Sizing::Hug,
        Some("fill") => Sizing::Fill,
        _ => Sizing::Fixed,
    }
}

/// Translate main-axis alignment, defaulting unknown values to the start.
fn align_main(v: Option<&str>) -> MainAlign {
    match v {
        Some("center") => MainAlign::Center,
        Some("end") => MainAlign::Max,
        Some("space-between") => MainAlign::SpaceBetween,
        Some("space-around") => MainAlign::SpaceAround,
        Some("space-evenly") => MainAlign::SpaceEvenly,
        _ => MainAlign::Min,
    }
}

/// Translate cross-axis alignment, including stretch and baseline.
fn align_cross(v: Option<&str>) -> CrossAlign {
    match v {
        Some("center") => CrossAlign::Center,
        Some("end") => CrossAlign::Max,
        _ => CrossAlign::Min,
    }
}

/// Read nonnegative padding supported by the current model.
fn padding(v: &Value) -> [f64; 4] {
    let Some(p) = v.get("layoutPadding") else { return [0.0; 4] };
    [number(p, "p1", 0.0), number(p, "p2", 0.0), number(p, "p3", 0.0), number(p, "p4", 0.0)].map(|n| n.clamp(0.0, 100_000.0))
}

/// Translate a supported Penpot track definition to native grid sizing.
fn grid_track(v: &Value) -> Option<GridTrack> {
    match string(v, "type")? {
        "fixed" => Some(GridTrack::fixed(number(v, "value", 0.0).max(0.0))),
        "auto" => Some(GridTrack::hug()),
        "flex" => Some(GridTrack::fill(number(v, "value", 1.0).max(0.001))),
        _ => None,
    }
}

/// Map Penpot layout fields to the FigCraft frame layout engine. Unsupported directions / custom
/// grid track types are intentionally left as absolute-positioned frames (not approximated).
fn layout_props(v: &Value, warnings: &mut Vec<String>) -> Option<AutoLayout> {
    let mode = string(v, "layout").or_else(|| string(v, "layoutMode"))?;
    let unsupported = if matches!(string(v, "layoutFlexDir"), Some("row-reverse" | "column-reverse")) {
        Some("reverse flow")
    } else if string(v, "layoutAlignItems") == Some("stretch") {
        Some("stretch alignment")
    } else if string(v, "layoutWrapType") == Some("wrap") && matches!(string(v, "layoutAlignContent"), Some("space-around" | "space-evenly")) {
        Some("wrapped-line distribution")
    } else if v.get("layoutPadding").is_some_and(|p| ["p1", "p2", "p3", "p4"].iter().any(|k| number(p, k, 0.0) < 0.0)) {
        Some("negative padding")
    } else if list(v, "layoutGridColumns").chain(list(v, "layoutGridRows")).any(|t| string(t, "type") == Some("percent")) {
        Some("percentage tracks")
    } else {
        None
    };
    if let Some(reason) = unsupported {
        warn(warnings, format!("Penpot {reason} is unsupported; saved positions and source metadata were kept"));
        return None;
    }
    let mut al = AutoLayout { padding: padding(v), strokes_included: false, ..AutoLayout::default() };
    let gap = v.get("layoutGap");
    let row_gap = gap.map(|v| number(v, "rowGap", 0.0)).unwrap_or(0.0).clamp(0.0, 100_000.0);
    let column_gap = gap.map(|v| number(v, "columnGap", 0.0)).unwrap_or(0.0).clamp(0.0, 100_000.0);
    al.main_align = align_main(string(v, "layoutJustifyContent"));
    al.cross_align = align_cross(string(v, "layoutAlignItems"));
    match mode {
        "flex" => {
            al.mode = match string(v, "layoutFlexDir") {
                Some("column") | Some("column-reverse") => LayoutMode::Vertical,
                Some("row") | Some("row-reverse") | None => LayoutMode::Horizontal,
                _ => {
                    warn(warnings, "Unknown Penpot flex direction retained at absolute positions".into());
                    return None;
                }
            };
            al.gap = if al.mode == LayoutMode::Horizontal { column_gap } else { row_gap };
            al.cross_gap = if al.mode == LayoutMode::Horizontal { row_gap } else { column_gap };
            al.wrap = matches!(string(v, "layoutWrapType"), Some("wrap"));
            if al.wrap {
                al.content_align = match string(v, "layoutAlignContent") {
                    Some("space-between") => ContentAlign::SpaceBetween,
                    _ => ContentAlign::Auto,
                };
            }
        }
        "grid" => {
            let cols = list(v, "layoutGridColumns").collect::<Vec<_>>();
            let rows = list(v, "layoutGridRows").collect::<Vec<_>>();
            if cols.is_empty() || rows.is_empty() || cols.len() > 256 || rows.len() > 256 {
                warn(warnings, "Unsupported Penpot grid dimensions; positions kept".into());
                return None;
            }
            let Some(column_tracks) = cols.iter().map(|v| grid_track(v)).collect::<Option<Vec<_>>>() else {
                warn(warnings, "Unsupported Penpot grid column tracks; positions kept".into());
                return None;
            };
            let Some(row_tracks) = rows.iter().map(|v| grid_track(v)).collect::<Option<Vec<_>>>() else {
                warn(warnings, "Unsupported Penpot grid row tracks; positions kept".into());
                return None;
            };
            al.mode = LayoutMode::Grid;
            al.main_align = align_main(string(v, "layoutJustifyItems"));
            al.grid_columns = column_tracks.len() as u32;
            al.grid_rows = row_tracks.len() as u32;
            al.column_tracks = column_tracks;
            al.row_tracks = row_tracks;
            al.gap = column_gap;
            al.cross_gap = row_gap;
        }
        _ => {
            warn(warnings, format!("Unsupported Penpot layout '{mode}' retained at absolute positions"));
            return None;
        }
    }
    Some(al)
}

/// Penpot grid-cell indices are one-based; FigCraft uses zero-based indices. The `shapes`
/// memberships are the authoritative mapping (not a cell's UUID).
fn grid_cells(parent: &Value) -> BTreeMap<&str, &Value> {
    let mut out = BTreeMap::new();
    for cell in parent.get("layoutGridCells").and_then(Value::as_object).into_iter().flat_map(|m| m.values()) {
        for id in list(cell, "shapes").filter_map(Value::as_str) {
            out.entry(id).or_insert(cell);
        }
    }
    out
}

/// Map explicit grid-cell alignment; absent values inherit the grid setting.
fn align_cell(v: Option<&str>) -> Option<CellAlign> {
    match v {
        Some("start") => Some(CellAlign::Min),
        Some("center") => Some(CellAlign::Center),
        Some("end") => Some(CellAlign::Max),
        _ => None,
    }
}

/// Apply sizing, margins, alignment and one-based grid-cell placement to a layer.
fn apply_child_layout(v: &Value, cell: Option<&Value>, node: &mut figcraft_doc::Node) {
    node.child_layout.sizing_h = penpot_sizing(string(v, "layoutItemHSizing"));
    node.child_layout.sizing_v = penpot_sizing(string(v, "layoutItemVSizing"));
    node.child_layout.absolute = v.get("layoutItemAbsolute").and_then(Value::as_bool) == Some(true);
    if v.get("layoutItemMargin").is_some_and(|m| ["m1", "m2", "m3", "m4"].iter().any(|k| number(m, k, 0.0) != 0.0))
        || string(v, "layoutItemAlignSelf").is_some_and(|a| a != "auto")
    {
        node.plugin_data.insert("penpot.unsupported.itemLayout".into(), "true".into());
    }
    for (key, target) in [
        ("layoutItemMinW", &mut node.child_layout.min_w),
        ("layoutItemMaxW", &mut node.child_layout.max_w),
        ("layoutItemMinH", &mut node.child_layout.min_h),
        ("layoutItemMaxH", &mut node.child_layout.max_h),
    ] {
        if v.get(key).is_some() {
            *target = Some(number(v, key, 0.0).max(0.0));
        }
    }
    if let Some(cell) = cell {
        // A manually positioned cell is not the same as an absolutely positioned child:
        // it remains in grid flow but has a fixed grid slot.
        node.child_layout.grid_column = cell.get("column").and_then(Value::as_u64).and_then(|n| n.checked_sub(1)).and_then(|n| u32::try_from(n).ok());
        node.child_layout.grid_row = cell.get("row").and_then(Value::as_u64).and_then(|n| n.checked_sub(1)).and_then(|n| u32::try_from(n).ok());
        node.child_layout.grid_column_span = cell.get("columnSpan").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(1).max(1);
        node.child_layout.grid_row_span = cell.get("rowSpan").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(1).max(1);
        node.child_layout.grid_align_h = align_cell(string(cell, "justifySelf"));
        node.child_layout.grid_align_v = align_cell(string(cell, "alignSelf"));
    }
}

/// Bring Penpot library color swatches into FigCraft's editable document palette.
/// Bind only the narrow subset of Penpot `appliedTokens` that FigCraft can represent
/// without changing the initial appearance: single solid fill/stroke colors and fixed
/// layer width/height. Penpot stores token names without their set; if two sets contain
/// the same name, choosing one would silently change the design when modes change.
/// Preserve *all* original bindings in `penpot.source.appliedTokens` for future importers.
const NUMBER_TOKEN_PROPERTIES: &[&str] =
    &["columnGap", "rowGap", "p1", "p2", "p3", "p4", "r1", "r2", "r3", "r4", "strokeWidth", "fontSize", "letterSpacing"];

/// Find a bindable native field only when its current value matches the token.
fn token_number_field(node: &figcraft_doc::Node, property: &str, value: f64) -> Option<&'static str> {
    let same = |current: f64| value.is_finite() && (current - value).abs() < 0.0001;
    let layout = node.layout.as_ref();
    let vertical = layout.is_some_and(|l| l.mode == LayoutMode::Vertical);
    let gap = |main: bool| {
        layout.and_then(|l| {
            let (field, current) = if main { ("gap", l.gap) } else { ("crossGap", l.cross_gap) };
            same(current).then_some(field)
        })
    };
    let padding = |i: usize, field| layout.and_then(|l| l.padding.get(i).copied()).filter(|p| same(*p)).map(|_| field);
    let radius = |i: usize, field| node.corners.radii.get(i).copied().filter(|r| same(*r)).map(|_| field);
    let text = match &node.kind {
        NodeKind::Text(t) => Some(&t.style),
        _ => None,
    };
    match property {
        "columnGap" => gap(!vertical),
        "rowGap" => gap(vertical),
        "p1" => padding(0, "paddingTop"),
        "p2" => padding(1, "paddingRight"),
        "p3" => padding(2, "paddingBottom"),
        "p4" => padding(3, "paddingLeft"),
        "r1" => radius(0, "topLeftRadius"),
        "r2" => radius(1, "topRightRadius"),
        "r3" => radius(2, "bottomRightRadius"),
        "r4" => radius(3, "bottomLeftRadius"),
        "strokeWidth" => (!node.strokes.is_empty() && same(node.stroke.weight)).then_some("strokeWeight"),
        "fontSize" => text.filter(|t| same(t.font_size)).map(|_| "fontSize"),
        "letterSpacing" => text.filter(|t| matches!(t.letter_spacing, LetterSpacing::Pixels(px) if same(px))).map(|_| "letterSpacing"),
        _ => None,
    }
}

/// Bind supported token references while retaining unresolved source metadata.
fn bind_applied_tokens(doc: &mut Document, active_modes: &BTreeMap<String, String>, warnings: &mut Vec<String>) {
    // Index names within the original Penpot file. Multiple sets may export tokens
    // with identical names; keep all candidates rather than silently selecting one.
    let mut index: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for var in &doc.variables.variables {
        let Some(raw) = var.id.strip_prefix("penpot:tokens:files/") else { continue };
        let Some((file_id, path)) = raw.split_once(':') else { continue };
        // Legacy ids include the set; themed ids contain just the token path.
        let short_name = var.id.strip_prefix(&format!("{}:", var.collection)).unwrap_or(path);
        for token_name in [short_name, var.name.as_str()] {
            index.entry((file_id.to_owned(), token_name.to_owned())).or_default().push(var.id.clone());
        }
    }
    if index.is_empty() {
        return;
    }

    let mut updates = Vec::new();
    let mut unlinked_by_node = Vec::new();
    let mut linked = 0usize;
    let mut rejected = 0usize;
    let mut affected = BTreeSet::new();
    // Work from immutable snapshots first to avoid mixing node mutation with
    // variable resolution and to keep this pass deterministic.
    for node in doc.nodes() {
        let (Some(file), Some(raw)) = (node.plugin_data.get("penpot.fileId"), node.plugin_data.get("penpot.source.appliedTokens")) else { continue };
        let Ok(Value::Object(tokens)) = serde_json::from_str::<Value>(raw) else {
            rejected += 1;
            affected.insert(node.name.clone());
            continue;
        };
        let mut bindings = BTreeMap::new();
        let mut unlinked = BTreeMap::new();
        for (property, token) in &tokens {
            let outcome = (|| {
                let raw_name = token.as_str().ok_or("not a token reference")?;
                let name = raw_name.strip_prefix('{').and_then(|x| x.strip_suffix('}')).unwrap_or(raw_name);
                if name.is_empty() || name.len() > 256 {
                    return Err("invalid token name");
                }
                let ids = index.get(&(file.clone(), name.to_owned())).ok_or("token not imported")?;
                let [id] = ids.as_slice() else { return Err("ambiguous token name") };
                let value = doc.variables.resolve(id, active_modes).ok_or("token does not resolve")?;
                let field = match (property.as_str(), value) {
                    ("fill", VariableValue::Color(color))
                        if node.fills.len() == 1
                            && node.fills.first().and_then(Paint::solid_color).is_some_and(|c| c.to_rgba8() == color.to_rgba8()) =>
                    {
                        "fills/0/color"
                    }
                    ("strokeColor", VariableValue::Color(color))
                        if node.strokes.len() == 1
                            && node.strokes.first().and_then(Paint::solid_color).is_some_and(|c| c.to_rgba8() == color.to_rgba8()) =>
                    {
                        "strokes/0/color"
                    }
                    ("fill" | "strokeColor", _) => return Err("layer color differs from the token"),
                    ("width", VariableValue::Number(value)) if value.is_finite() && (node.width - value).abs() < 0.0001 => "width",
                    ("height", VariableValue::Number(value)) if value.is_finite() && (node.height - value).abs() < 0.0001 => "height",
                    (property, VariableValue::Number(value)) => {
                        token_number_field(node, property, value).ok_or(if NUMBER_TOKEN_PROPERTIES.contains(&property) {
                            "layer value differs from the token"
                        } else {
                            "property has no FigCraft binding"
                        })?
                    }
                    ("fontFamily", VariableValue::String(family)) if matches!(&node.kind, NodeKind::Text(t) if t.style.font_family == family) => {
                        "fontFamily"
                    }
                    _ => return Err("property has no FigCraft binding"),
                };
                Ok((field, id.clone()))
            })();
            match outcome {
                Ok((field, id)) => {
                    bindings.insert(field.to_owned(), id);
                    linked += 1;
                }
                Err(reason) => {
                    unlinked.insert(property.clone(), Value::from(reason));
                    rejected += 1;
                    affected.insert(node.name.clone());
                }
            }
        }
        if !unlinked.is_empty()
            && let Ok(json) = serde_json::to_string(&unlinked)
        {
            unlinked_by_node.push((node.id, json));
        }
        if !bindings.is_empty() {
            updates.push((node.id, bindings));
        }
    }
    for (node_id, bindings) in updates {
        if let Some(node) = doc.node_mut(node_id) {
            node.bound.extend(bindings);
        }
    }
    for (node_id, json) in unlinked_by_node {
        if let Some(node) = doc.node_mut(node_id) {
            node.plugin_data.insert("penpot.unlinkedTokens".into(), json);
        }
    }
    if linked > 0 {
        warn(
            warnings,
            format!(
                "Linked {linked} Penpot token reference(s) to native FigCraft properties (colors, sizes, gaps, padding, radii, stroke widths, font sizes and families)"
            ),
        );
    }
    if rejected > 0 {
        let examples: Vec<String> = affected.into_iter().take(4).map(|s| s.chars().take(60).collect()).collect();
        warn(
            warnings,
            format!(
                "{rejected} Penpot token reference(s) could not be safely linked (unsupported, ambiguous or different resolved value); original references retained on layers{}",
                if examples.is_empty() { String::new() } else { format!(" including {}", examples.join(", ")) }
            ),
        );
    }
}

/// Import Penpot library colors as reusable native styles. Original design-token
/// references are handled separately by `bind_applied_tokens` after shape import.
fn import_document_colors(z: &mut ZipArchive<Cursor<&[u8]>>, base: &str, doc: &mut Document, warnings: &mut Vec<String>, budget: &mut InflateBudget) {
    let prefix = format!("{base}/colors/");
    let paths = z
        .file_names()
        .filter(|name| name.strip_prefix(&prefix).is_some_and(|part| part.len() == 41 && part.ends_with(".json") && part.get(..36).is_some_and(uuid)))
        .take(2049)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if paths.len() > 2048 {
        warn(warnings, "Penpot color library exceeds 2048 swatches; truncated".into());
    }
    for path in paths.into_iter().take(2048) {
        if let Ok(value) = json(z, &path, budget) {
            if let Some(color) = string(&value, "color").and_then(Color::from_hex) {
                let color = color.with_alpha((color.a as f64 * number(&value, "opacity", 1.0).clamp(0.0, 1.0)) as f32);
                if !doc.document_colors.contains(&color) {
                    doc.document_colors.push(color);
                }
                if doc.styles.len() < 8192
                    && let (Some(id), Some(name)) = (string(&value, "id").filter(|s| uuid(s)), string(&value, "name"))
                {
                    let full_name = match string(&value, "path").filter(|s| !s.is_empty()) {
                        Some(path) => format!("{path}/{name}"),
                        None => name.to_owned(),
                    };
                    doc.styles.push(Style {
                        id: format!("penpot:color:{base}:{id}"),
                        name: full_name.chars().take(256).collect(),
                        description: "Imported from Penpot color library".into(),
                        kind: StyleKind::Paint { paints: vec![Paint::solid(color)] },
                        bound: BTreeMap::new(),
                    });
                }
            } else if value.get("gradient").is_some() || value.get("image").is_some() {
                warn(warnings, "Gradient/image library swatches kept on imported shapes; not added to the solid color palette".into());
            }
        }
    }
}

/// Import named Penpot typography assets as native reusable FigCraft text styles.
/// Keep original values on the text nodes: attaching a shared style automatically
/// would override local formatting unless the source explicitly links it.
fn import_typography_styles(
    z: &mut ZipArchive<Cursor<&[u8]>>,
    base: &str,
    doc: &mut Document,
    warnings: &mut Vec<String>,
    budget: &mut InflateBudget,
) -> Vec<Value> {
    let mut custom_refs = Vec::new();
    let prefix = format!("{base}/typographies/");
    let paths: Vec<String> = z
        .file_names()
        .filter(|name| name.strip_prefix(&prefix).is_some_and(|part| part.len() == 41 && part.ends_with(".json") && part.get(..36).is_some_and(uuid)))
        .take(2049)
        .map(str::to_owned)
        .collect();
    if paths.len() > 2048 {
        warn(warnings, "Penpot typography library exceeds 2048 entries; truncated".into());
    }
    for path in paths.into_iter().take(2048) {
        if doc.styles.len() >= 8192 {
            warn(warnings, "Imported Penpot styles capped at 8192 entries".into());
            break;
        }
        let Ok(value) = json(z, &path, budget) else {
            warn(warnings, "A Penpot typography definition could not be decoded".into());
            continue;
        };
        let Some(id) = string(&value, "id").filter(|s| uuid(s)) else {
            continue;
        };
        let Some(name) = string(&value, "name").filter(|s| !s.is_empty()) else {
            continue;
        };
        let name = match string(&value, "path").filter(|s| !s.is_empty()) {
            Some(path) => format!("{path}/{name}"),
            None => name.to_owned(),
        };
        // Typography-library assets can reference a Penpot-managed font that
        // has no matching text run yet. Retain that reference on the page for
        // diagnosing missing/custom families without claiming to own its bytes.
        for reference in referenced_custom_fonts(&value) {
            if custom_refs.len() < 128 && !custom_refs.contains(&reference) {
                custom_refs.push(reference);
            }
        }
        let style = text_style(&value, &TextStyle::default());
        doc.styles.push(Style {
            id: format!("penpot:text:{base}:{id}"),
            name: name.chars().take(256).collect(),
            description: "Imported from Penpot typography library".into(),
            kind: StyleKind::Text { style },
            bound: BTreeMap::new(),
        });
    }
    custom_refs
}

/// DTCG typography and shadow objects are not scalar FigCraft variables. Instead, bring
/// simple literal compound values across as reusable native styles. In particular, do
/// not evaluate token aliases here: active themes may map those to different values.
fn compound_number(value: &Value) -> Option<f64> {
    let n =
        value.as_f64().or_else(|| value.as_str()?.trim().strip_suffix("px").or_else(|| value.as_str()).and_then(|s| s.trim().parse::<f64>().ok()))?;
    (n.is_finite() && n.abs() <= 100_000.0).then_some(n)
}

/// Convert literal typography or shadow token values to a reusable native style.
fn composite_token_style(token: &Value) -> Option<StyleKind> {
    let kind = string(token, "type").or_else(|| string(token, "$type"))?;
    let value = token.get("value").or_else(|| token.get("$value"))?;
    match kind {
        "typography" => {
            let props = value.as_object()?;
            if props.is_empty() || (!props.contains_key("fontFamily") && !props.contains_key("fontSize")) {
                return None;
            }
            let mut style = TextStyle::default();
            let mut fields = serde_json::Map::new();
            if let Some(family) = value.get("fontFamily").and_then(Value::as_str) {
                if family.contains('{') || family.len() > 200 {
                    return None;
                }
                fields.insert("fontFamily".into(), Value::String(family.into()));
            }
            if let Some(size) = value.get("fontSize").and_then(compound_number) {
                if size <= 0.0 || size > 1000.0 {
                    return None;
                }
                fields.insert("fontSize".into(), serde_json::json!(size));
            }
            if let Some(weight) = value.get("fontWeight") {
                if weight.as_str().is_some_and(|s| s.contains('{')) {
                    return None;
                }
                fields.insert("fontWeight".into(), Value::String(weight.as_str().map(str::to_owned).unwrap_or_else(|| weight.to_string())));
            }
            if let Some(spacing_value) = value.get("letterSpacing") {
                fields.insert("letterSpacing".into(), serde_json::json!(compound_number(spacing_value)?));
            }
            if let Some(raw_line) = value.get("lineHeight") {
                let line = compound_number(raw_line)?;
                if line > 0.0 {
                    // DTCG line height may be a unitless multiplier or a pixel height.
                    // Do not guess: explicit pixel lengths are native pixels; an
                    // unadorned small numeric value is a font-size multiplier.
                    if value.get("lineHeight").and_then(Value::as_str).is_some_and(|s| s.trim().ends_with("px")) {
                        style.line_height = LineHeight::Pixels(line);
                    } else if line <= 10.0 {
                        fields.insert("lineHeight".into(), serde_json::json!(line));
                    } else {
                        return None;
                    }
                }
            }
            let parsed = text_style(&Value::Object(fields), &style);
            Some(StyleKind::Text { style: parsed })
        }
        "boxShadow" | "shadow" => {
            let parts: Vec<&Value> = if let Some(items) = value.as_array() { items.iter().take(32).collect() } else { vec![value] };
            if parts.is_empty() {
                return None;
            }
            let mut effects = Vec::new();
            for part in parts {
                let raw = part.as_object()?;
                let color = token_color(raw.get("color")?.as_str()?)?;
                let x = raw.get("x").or_else(|| raw.get("offsetX")).and_then(compound_number)?;
                let y = raw.get("y").or_else(|| raw.get("offsetY")).and_then(compound_number)?;
                let blur = raw.get("blur").and_then(compound_number)?;
                let spread = match raw.get("spread") {
                    Some(value) => compound_number(value)?,
                    None => 0.0,
                };
                if blur < 0.0 {
                    return None;
                }
                let shadow = Shadow { color, offset: (x, y), radius: blur, spread, ..Shadow::default() };
                let inner = matches!(string(part, "type"), Some("innerShadow" | "inner" | "inset"));
                effects.push(Effect { kind: if inner { EffectKind::InnerShadow(shadow) } else { EffectKind::DropShadow(shadow) }, visible: true });
            }
            Some(StyleKind::Effect { effects })
        }
        _ => None,
    }
}

/// Import compound token styles without guessing unresolved aliases.
fn import_composite_token_styles(source: &Value, base: &str, doc: &mut Document, warnings: &mut Vec<String>) {
    let Some(sets) = source.get("sets").and_then(Value::as_object) else { return };
    let mut count = 0;
    let mut unsupported = 0;
    for (set_name, definition) in sets.iter().take(256) {
        let Some(tokens) = definition.get("tokens") else { continue };
        let mut flattened = Vec::new();
        flatten_tokens(tokens, "", &mut flattened, 0);
        for (name, token) in flattened {
            let kind = string(token, "$type").or_else(|| string(token, "type"));
            if !matches!(kind, Some("typography" | "boxShadow" | "shadow")) {
                continue;
            }
            if doc.styles.len() >= 8192 {
                unsupported += 1;
                continue;
            }
            if let Some(kind) = composite_token_style(token) {
                doc.styles.push(Style {
                    id: format!("penpot:token-style:{base}:{set_name}:{name}"),
                    name: format!("Penpot tokens/{set_name}/{name}").chars().take(256).collect(),
                    description: "Imported from a literal compound Penpot token; theme aliases remain source metadata".into(),
                    kind,
                    bound: BTreeMap::new(),
                });
                count += 1;
            } else {
                unsupported += 1;
            }
        }
    }
    if count > 0 {
        warn(warnings, format!("Imported {count} literal Penpot typography/shadow tokens as reusable styles"));
    }
    if unsupported > 0 {
        warn(warnings, format!("{unsupported} compound tokens could not be converted safely; source JSON preserved when possible"));
    }
}

/// Convert scalar DTCG/Penpot tokens to editable native variables. Compound
/// typography, shadow, dimension expressions and unresolved aliases are deliberately
/// not interpreted as literal values; the entire original library is preserved on
/// a page for a later, fuller parser.
fn scalar_token(token: &Value) -> Option<(VariableType, VariableValue)> {
    let kind = string(token, "type").or_else(|| string(token, "$type"))?;
    let value = token.get("value").or_else(|| token.get("$value"))?;
    let text = value.as_str();
    if text.is_some_and(|t| t.starts_with('{') && t.ends_with('}')) {
        return None;
    }
    match kind {
        "color" => Some((VariableType::Color, VariableValue::Color(token_color(text?)?))),
        "number" | "numeric" | "dimension" | "sizing" | "spacing" | "borderRadius" | "opacity" | "fontSizes" | "fontWeights" | "borderWidth"
        | "letterSpacing" | "rotation" | "lineHeights" | "paragraphSpacing" => {
            let n = value.as_f64().or_else(|| text.and_then(|t| t.strip_suffix("px").unwrap_or(t).trim().parse::<f64>().ok()))?;
            if !n.is_finite() || n.abs() > 1_000_000.0 {
                return None;
            }
            Some((VariableType::Number, VariableValue::Number(n)))
        }
        "string" | "fontFamily" | "fontFamilies" | "fontWeight" | "textCase" | "textDecoration" => {
            let text = text?;
            if text.len() > 4096 {
                return None;
            }
            Some((VariableType::String, VariableValue::String(text.into())))
        }
        "boolean" => Some((VariableType::Boolean, VariableValue::Boolean(value.as_bool()?))),
        _ => None,
    }
}

/// Native alias variables must have the same type as their target. Penpot's
/// token type vocabulary is larger than FigCraft's four scalar types; unknown
/// kinds remain source metadata rather than being converted incorrectly.
fn token_color(text: &str) -> Option<Color> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("transparent") {
        return Some(Color::new(0.0, 0.0, 0.0, 0.0));
    }
    if let Some(args) = text.strip_prefix("rgba(").or_else(|| text.strip_prefix("rgb(")).and_then(|t| t.strip_suffix(')')) {
        let parts: Vec<f64> = args
            .split([',', ' ', '/'])
            .filter(|p| !p.is_empty())
            .map(|p| p.trim().parse::<f64>().ok().filter(|n| n.is_finite()))
            .collect::<Option<_>>()?;
        let channel = |i: usize| parts.get(i).map(|v| (v / 255.0).clamp(0.0, 1.0) as f32);
        let alpha = parts.get(3).map_or(1.0, |a| a.clamp(0.0, 1.0) as f32);
        return Some(Color::new(channel(0)?, channel(1)?, channel(2)?, alpha));
    }
    Color::from_hex(text)
}

/// Map supported scalar token types to the native variable model.
fn token_kind(token: &Value) -> Option<VariableType> {
    match string(token, "type").or_else(|| string(token, "$type"))? {
        "color" => Some(VariableType::Color),
        "number" | "numeric" | "dimension" | "sizing" | "spacing" | "borderRadius" | "opacity" | "fontSizes" | "fontWeights" | "borderWidth"
        | "letterSpacing" | "rotation" | "lineHeights" | "paragraphSpacing" => Some(VariableType::Number),
        "string" | "fontFamily" | "fontFamilies" | "fontWeight" | "textCase" | "textDecoration" => Some(VariableType::String),
        "boolean" => Some(VariableType::Boolean),
        _ => None,
    }
}

/// A Penpot alias is an entire value of the form `{path.to.token}`, not a
/// string containing an interpolated expression. Do not interpret references
/// outside the same token set as a live FigCraft variable without a mode map.
fn token_alias(token: &Value) -> Option<&str> {
    let text = token.get("value").or_else(|| token.get("$value"))?.as_str()?;
    let path = text.strip_prefix('{')?.strip_suffix('}')?;
    (!path.is_empty() && path.len() <= 256 && !path.contains('{') && !path.contains('}')).then_some(path)
}

/// Resolve the declared type through a same-set alias chain. Guard cycles and
/// excessive depth; all terminal scalar values must parse successfully.
fn resolved_token_kind(name: &str, tokens: &BTreeMap<&str, &Value>, visiting: &mut BTreeSet<String>, depth: usize) -> Option<VariableType> {
    if depth >= 32 || !visiting.insert(name.to_owned()) {
        return None;
    }
    let result = (|| {
        let token = *tokens.get(name)?;
        let kind = token_kind(token)?;
        if let Some(next) = token_alias(token) {
            (resolved_token_kind(next, tokens, visiting, depth + 1)? == kind).then_some(kind)
        } else {
            (scalar_token(token)?.0 == kind).then_some(kind)
        }
    })();
    visiting.remove(name);
    result
}

/// Collect dotted token names within the nesting and token-count budgets.
fn flatten_tokens<'a>(value: &'a Value, path: &str, out: &mut Vec<(String, &'a Value)>, depth: usize) {
    if depth >= 16 || out.len() >= 4096 {
        return;
    }
    if value.get("value").is_some() || value.get("$value").is_some() {
        if !path.is_empty() {
            out.push((path.into(), value));
        }
        return;
    }
    let Some(fields) = value.as_object() else {
        return;
    };
    for (name, token) in fields {
        if out.len() >= 4096 {
            break;
        }
        if name.starts_with('$') || name == "id" || name == "name" || name == "type" {
            continue;
        }
        let full = if path.is_empty() { name.to_string() } else { format!("{path}.{name}") };
        if full.len() <= 256 {
            flatten_tokens(token, &full, out, depth + 1);
        }
    }
}

/// Each Penpot token set becomes a native collection with a Default mode. Sets
/// and themes can overlap, so we do not pretend that independent set variables
/// are equivalent to multidimensional Penpot theme activation.
/// Total expression work allowed across every token and theme mode in one import.
/// Charging the whole expression at every evaluation bounds repeated alias expansion.
const MAX_TOKEN_RESOLUTION_WORK: usize = 2_000_000;

struct TokenResolutionBudget {
    remaining: usize,
}

impl TokenResolutionBudget {
    fn spend(&mut self, units: usize) -> Option<()> {
        match self.remaining.checked_sub(units) {
            Some(left) => {
                self.remaining = left;
                Some(())
            }
            None => {
                self.remaining = 0;
                None
            }
        }
    }
}

fn token_number(
    path: &str,
    sets: &[&str],
    tokens: &BTreeMap<&str, BTreeMap<String, &Value>>,
    alias_depth: usize,
    expr_depth: usize,
    budget: &mut TokenResolutionBudget,
) -> Option<f64> {
    // Each alias lookup costs work, even when it fails. Do not give aliases a fresh budget.
    budget.spend(1)?;
    if alias_depth > 32 || expr_depth >= MAX_TOKEN_EXPR_NESTING {
        return None;
    }
    // Every examined set is work, including misses. Charge the actual linear scan,
    // rather than one unit for the entire lookup.
    let token = sets.iter().rev().find_map(|s| {
        budget.spend(1)?;
        tokens.get(s)?.get(path)
    })?;
    let value = token.get("$value").or_else(|| token.get("value"))?;
    let number = match value {
        Value::Number(n) => n.as_f64(),
        Value::String(text) => {
            // Parsing a string is linear in its length. Re-parsing an alias costs it again.
            budget.spend(text.len())?;
            let mut resolve = |r: &str, nesting: usize| token_number(r, sets, tokens, alias_depth + 1, nesting, budget);
            TokenExpr { text: text.as_bytes(), at: 0, nesting: expr_depth, resolve: &mut resolve }.parse()
        }
        _ => None,
    }?;
    (number.is_finite() && number.abs() <= 1_000_000.0).then_some(number)
}

/// Maximum call nesting while evaluating untrusted Penpot expressions, including aliases.
const MAX_TOKEN_EXPR_NESTING: usize = 64;

struct TokenExpr<'a> {
    text: &'a [u8],
    at: usize,
    nesting: usize,
    resolve: &'a mut dyn FnMut(&str, usize) -> Option<f64>,
}

impl TokenExpr<'_> {
    fn parse(mut self) -> Option<f64> {
        let value = self.sum()?;
        self.skip();
        (self.at == self.text.len()).then_some(value)
    }
    fn skip(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }
    fn eat(&mut self, c: u8) -> bool {
        self.skip();
        let found = self.text.get(self.at) == Some(&c);
        if found {
            self.at += 1;
        }
        found
    }
    fn sum(&mut self) -> Option<f64> {
        let mut value = self.product()?;
        loop {
            if self.eat(b'+') {
                value += self.product()?;
            } else if self.eat(b'-') {
                value -= self.product()?;
            } else {
                return Some(value);
            }
        }
    }
    fn product(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        loop {
            if self.eat(b'*') {
                value *= self.unary()?;
            } else if self.eat(b'/') {
                let d = self.unary()?;
                value = if d == 0.0 { return None } else { value / d };
            } else if self.eat(b'%') {
                let d = self.unary()?;
                value = if d == 0.0 { return None } else { value % d };
            } else {
                return Some(value);
            }
        }
    }
    fn unary(&mut self) -> Option<f64> {
        if self.nesting >= MAX_TOKEN_EXPR_NESTING {
            return None;
        }
        self.nesting += 1;
        let value = if self.eat(b'-') {
            self.unary().map(|v| -v)
        } else {
            self.eat(b'+');
            self.atom()
        };
        self.nesting -= 1;
        value
    }
    fn atom(&mut self) -> Option<f64> {
        self.skip();
        if self.eat(b'(') {
            let value = self.sum()?;
            return self.eat(b')').then_some(value);
        }
        if self.eat(b'{') {
            let start = self.at;
            while self.text.get(self.at).is_some_and(|c| *c != b'}') {
                self.at += 1;
            }
            let name = std::str::from_utf8(self.text.get(start..self.at)?).ok()?.trim().to_owned();
            self.at += 1;
            return (self.resolve)(&name, self.nesting);
        }
        let start = self.at;
        while self.text.get(self.at).is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
            self.at += 1;
        }
        if self.at > start {
            let number = std::str::from_utf8(self.text.get(start..self.at)?).ok()?.parse::<f64>().ok()?;
            if self.text.get(self.at..self.at + 2) == Some(b"px") {
                self.at += 2;
            }
            return Some(number);
        }
        while self.text.get(self.at).is_some_and(u8::is_ascii_alphabetic) {
            self.at += 1;
        }
        let name = std::str::from_utf8(self.text.get(start..self.at)?).ok()?.to_ascii_lowercase();
        if !self.eat(b'(') {
            return None;
        }
        let mut args = vec![self.sum()?];
        while self.eat(b',') {
            args.push(self.sum()?);
        }
        if !self.eat(b')') {
            return None;
        }
        let first = *args.first()?;
        Some(match (name.as_str(), args.len()) {
            ("round", 1) => first.round(),
            ("round" | "roundto", 2) => {
                let step = 10f64.powi(args[1] as i32);
                (first * step).round() / step
            }
            ("floor", 1) => first.floor(),
            ("ceil", 1) => first.ceil(),
            ("abs", 1) => first.abs(),
            ("sqrt", 1) if first >= 0.0 => first.sqrt(),
            ("log", 1) if first > 0.0 => first.ln(),
            ("log", 2) if first > 0.0 && args[1] > 0.0 && args[1] != 1.0 => first.log(args[1]),
            ("pow", 2) => first.powf(args[1]),
            ("min", _) => args.iter().copied().fold(f64::INFINITY, f64::min),
            ("max", _) => args.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            _ => return None,
        })
    }
}

type ModePlan<'a> = (String, String, Vec<&'a str>);
type CollectionPlan<'a> = (String, String, Vec<ModePlan<'a>>);

struct TokenTheme<'a> {
    id: String,
    name: &'a str,
    sets: Vec<&'a str>,
}

/// Look up a path through its existing set memberships, newest set first.
/// Avoid scanning every enabled set for every variable and mode; charge each
/// candidate examined so even deliberately overlapping sets have bounded work.
fn indexed_token<'a>(
    path: &str,
    selected: &BTreeSet<usize>,
    index: &'a BTreeMap<&str, Vec<(usize, &'a Value)>>,
    budget: &mut TokenResolutionBudget,
) -> Option<&'a Value> {
    index.get(path)?.iter().rev().find_map(|(set, value)| {
        budget.spend(1)?;
        selected.contains(set).then_some(*value)
    })
}

/// The themes of a token file by group, each with the token sets it turns on (of those in `tokens`).
fn token_theme_groups<'a>(value: &'a Value, tokens: &BTreeMap<&str, BTreeMap<String, &Value>>) -> Vec<(String, Vec<TokenTheme<'a>>)> {
    let mut groups: Vec<(String, Vec<TokenTheme>)> = Vec::new();
    for theme in list(value, "$themes").take(256) {
        let Some(name) = string(theme, "name") else { continue };
        let group = string(theme, "group").filter(|g| !g.trim().is_empty()).unwrap_or("Themes").to_owned();
        let sets = theme
            .get("selectedTokenSets")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter(|(set, state)| state.as_str().is_some_and(|s| s == "enabled" || s == "source") && tokens.contains_key(set.as_str()))
                    .map(|(set, _)| set.as_str())
                    .collect()
            })
            .unwrap_or_default();
        let id = string(theme, "id").unwrap_or(name).chars().take(80).collect();
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, themes)) => themes.push(TokenTheme { id, name, sets }),
            None => groups.push((group, vec![TokenTheme { id, name, sets }])),
        }
    }
    groups
}

/// Drops the resolved tokens that alias a variable which isn't being made, until none does; how
/// many were dropped.
fn drop_dangling_aliases(
    resolved: &mut BTreeMap<&str, (usize, VariableType, BTreeMap<String, VariableValue>)>,
    var_id: &dyn Fn(&str) -> String,
) -> usize {
    let mut dropped = 0;
    loop {
        let ids: BTreeSet<String> = resolved.keys().map(|p| var_id(p)).collect();
        let dangling: Vec<&str> = resolved
            .iter()
            .filter(|(_, (_, _, values))| values.values().any(|v| matches!(v, VariableValue::Alias(target) if !ids.contains(target))))
            .map(|(path, _)| *path)
            .collect();
        if dangling.is_empty() {
            return dropped;
        }
        dropped += dangling.len();
        for path in dangling {
            resolved.remove(path);
        }
    }
}

/// Moves a base token whose expression refers to a themed token into that token's collection, so
/// it can follow the theme, until nothing moves (64 rounds at most); false when the lookup budget
/// ran out.
fn move_referencing_tokens(
    owner: &mut BTreeMap<String, usize>,
    base_enabled: &BTreeSet<usize>,
    path_index: &BTreeMap<&str, Vec<(usize, &Value)>>,
    resolution_budget: &mut TokenResolutionBudget,
) -> bool {
    for _ in 0..owner.len().min(64) {
        let moves: Vec<(String, usize)> = owner
            .iter()
            .filter(|(_, index)| **index == 0)
            .filter_map(|(path, _)| {
                resolution_budget.spend(1)?;
                let token = indexed_token(path, base_enabled, path_index, resolution_budget)?;
                if token_alias(token).is_some() {
                    return None;
                }
                let text = token.get("$value").or_else(|| token.get("value"))?.as_str()?;
                resolution_budget.spend(text.len())?;
                text.split('{')
                    .skip(1)
                    .filter_map(|part| part.split_once('}').map(|(r, _)| r.trim()))
                    .find_map(|r| owner.get(r).copied().filter(|i| *i != 0))
                    .map(|i| (path.clone(), i))
            })
            .collect();
        if resolution_budget.remaining == 0 {
            return false;
        }
        if moves.is_empty() {
            break;
        }
        owner.extend(moves);
    }
    true
}

/// Each token path's variable type, from its newest set that gives one; `None` when the lookup
/// budget ran out.
fn token_kinds<'o>(
    owner: &'o BTreeMap<String, usize>,
    path_index: &BTreeMap<&str, Vec<(usize, &Value)>>,
    resolution_budget: &mut TokenResolutionBudget,
) -> Option<BTreeMap<&'o str, VariableType>> {
    let mut kinds = BTreeMap::new();
    for path in owner.keys() {
        let kind = path_index.get(path.as_str()).and_then(|occurrences| {
            occurrences.iter().rev().find_map(|(_, token)| {
                resolution_budget.spend(1)?;
                token_kind(token)
            })
        });
        if resolution_budget.remaining == 0 {
            return None;
        }
        if let Some(kind) = kind {
            kinds.insert(path.as_str(), kind);
        }
    }
    Some(kinds)
}

/// Import token sets and theme groups as variables, collections and modes.
fn import_themed_tokens(
    value: &Value,
    base: &str,
    doc: &mut Document,
    active_modes: &mut BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) -> bool {
    let Some(root) = value.as_object() else { return false };
    let order: Vec<&str> = value.get("$metadata").map(|m| list(m, "tokenSetOrder").filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut set_names: Vec<&str> = root.iter().filter(|(k, v)| !k.starts_with('$') && v.is_object()).map(|(k, _)| k.as_str()).collect();
    if set_names.is_empty() {
        return false;
    }
    set_names.sort_by_key(|name| order.iter().position(|o| o == name).unwrap_or(usize::MAX));
    let set_names: Vec<&str> = set_names.into_iter().take(512).collect();
    let mut tokens: BTreeMap<&str, BTreeMap<String, &Value>> = BTreeMap::new();
    for name in &set_names {
        let mut flat = Vec::new();
        if let Some(tree) = root.get(*name) {
            flatten_tokens(tree, "", &mut flat, 0);
        }
        tokens.insert(name, flat.into_iter().collect());
    }
    // Store only sets that actually define a path, in precedence order.
    let set_index: BTreeMap<&str, usize> = set_names.iter().enumerate().map(|(i, s)| (*s, i)).collect();
    let mut path_index: BTreeMap<&str, Vec<(usize, &Value)>> = BTreeMap::new();
    for (set_id, name) in set_names.iter().enumerate() {
        for (path, value) in tokens.get(name).into_iter().flat_map(|set| set.iter()) {
            path_index.entry(path.as_str()).or_default().push((set_id, *value));
        }
    }
    let groups = token_theme_groups(value, &tokens);
    let (switching, constant): (Vec<_>, Vec<_>) = groups.into_iter().partition(|(_, themes)| themes.len() > 1);
    let themed_sets: BTreeSet<&str> = switching.iter().flat_map(|(_, t)| t.iter().flat_map(|t| t.sets.iter().copied())).collect();
    let active_sets: Vec<&str> = value.get("$metadata").map(|m| list(m, "activeSets").filter_map(Value::as_str).collect()).unwrap_or_default();
    let base_sets: BTreeSet<&str> = if switching.is_empty() && constant.is_empty() {
        set_names.iter().copied().filter(|s| active_sets.is_empty() || active_sets.contains(s)).collect()
    } else {
        constant
            .iter()
            .flat_map(|(_, t)| t.iter().flat_map(|t| t.sets.iter().copied()))
            .chain(active_sets.iter().copied().filter(|s| !themed_sets.contains(s)))
            .filter(|s| tokens.contains_key(s))
            .collect()
    };
    let ordered = |sets: &BTreeSet<&str>| set_names.iter().copied().filter(|s| sets.contains(s)).collect::<Vec<_>>();
    let base_order = ordered(&base_sets);
    let collection_id = |group: &str| format!("penpot:tokens:{base}:collection:{group}");
    let mut plans: Vec<CollectionPlan> =
        vec![(collection_id("base"), "Tokens".into(), vec![(format!("{}:default", collection_id("base")), "Default".into(), base_order.clone())])];
    for (group, themes) in &switching {
        let cid = collection_id(group);
        let modes = themes
            .iter()
            .map(|t| {
                let sets: BTreeSet<&str> = base_sets.iter().copied().chain(t.sets.iter().copied()).collect();
                (format!("{cid}:{}", t.id), t.name.chars().take(120).collect(), ordered(&sets))
            })
            .collect();
        plans.push((cid, group.chars().take(120).collect(), modes));
    }
    let mode_sets: BTreeMap<&str, BTreeSet<usize>> = plans
        .iter()
        .flat_map(|(_, _, modes)| {
            modes.iter().map(|(id, _, sets)| (id.as_str(), sets.iter().filter_map(|name| set_index.get(name).copied()).collect()))
        })
        .collect();
    let base_enabled: BTreeSet<usize> = base_order.iter().filter_map(|name| set_index.get(name).copied()).collect();
    // One budget across all modes, kinds, descriptions and token expressions.
    let mut resolution_budget = TokenResolutionBudget { remaining: MAX_TOKEN_RESOLUTION_WORK };
    let mut owner: BTreeMap<String, usize> = BTreeMap::new();
    for (index, (_, _, modes)) in plans.iter().enumerate().rev() {
        for (_, _, sets) in modes {
            for set in sets {
                if index > 0 && base_sets.contains(set) {
                    continue;
                }
                for path in tokens.get(set).into_iter().flat_map(|t| t.keys()) {
                    if resolution_budget.spend(1).is_none() {
                        warn(warnings, "Penpot token lookup work budget exhausted; skipped themed tokens".into());
                        return false;
                    }
                    owner.insert(path.clone(), index);
                }
            }
        }
    }
    if !move_referencing_tokens(&mut owner, &base_enabled, &path_index, &mut resolution_budget) {
        warn(warnings, "Penpot token lookup work budget exhausted; skipped themed tokens".into());
        return false;
    }
    let var_id = |path: &str| format!("penpot:tokens:{base}:{path}");
    let Some(kinds) = token_kinds(&owner, &path_index, &mut resolution_budget) else {
        warn(warnings, "Penpot token lookup work budget exhausted; skipped themed tokens".into());
        return false;
    };
    let mut resolved: BTreeMap<&str, (usize, VariableType, BTreeMap<String, VariableValue>)> = BTreeMap::new();
    let mut skipped = 0usize;
    for (path, &index) in &owner {
        let Some((_, _, modes)) = plans.get(index) else { continue };
        let Some(&kind) = kinds.get(path.as_str()) else {
            skipped += 1;
            continue;
        };
        let mut values = BTreeMap::new();
        for (mode_id, _, sets) in modes {
            let Some(selected) = mode_sets.get(mode_id.as_str()) else { continue };
            let Some(token) = indexed_token(path, selected, &path_index, &mut resolution_budget) else { continue };
            let value = match token_alias(token) {
                Some(target) if kinds.get(target) == Some(&kind) => Some(VariableValue::Alias(var_id(target))),
                Some(_) => None,
                None => scalar_token(token).filter(|(k, _)| *k == kind).map(|(_, v)| v).or_else(|| {
                    (kind == VariableType::Number)
                        .then(|| token_number(path, sets, &tokens, 0, 0, &mut resolution_budget))
                        .flatten()
                        .map(VariableValue::Number)
                }),
            };
            if let Some(value) = value {
                values.insert(mode_id.clone(), value);
            }
        }
        if resolution_budget.remaining == 0 {
            warn(warnings, "Penpot token lookup work budget exhausted; skipped themed tokens".into());
            return false;
        }
        let Some(fallback) = modes.iter().find_map(|(m, _, _)| values.get(m).cloned()) else {
            skipped += 1;
            continue;
        };
        for (mode_id, _, _) in modes {
            values.entry(mode_id.clone()).or_insert_with(|| fallback.clone());
        }
        resolved.insert(path.as_str(), (index, kind, values));
    }
    skipped += drop_dangling_aliases(&mut resolved, &var_id);
    let mut converted = 0usize;
    let mut used_collections = BTreeSet::new();
    for (path, (index, kind, values)) in resolved {
        if doc.variables.variables.len() >= 8192 {
            skipped += 1;
            continue;
        }
        let Some((collection, group, _)) = plans.get(index) else { continue };
        let description = path_index
            .get(path)
            .and_then(|occurrences| {
                occurrences.iter().rev().find_map(|(_, token)| {
                    resolution_budget.spend(1)?;
                    string(token, "$description").or_else(|| string(token, "description"))
                })
            })
            .unwrap_or_default();
        if resolution_budget.remaining == 0 {
            warn(warnings, "Penpot token lookup work budget exhausted; remaining descriptions were skipped".into());
            break;
        }
        doc.variables.variables.push(Variable {
            id: var_id(path),
            name: format!("{group}/{path}"),
            collection: collection.clone(),
            kind,
            values,
            description: description.chars().take(1024).collect(),
            scopes: None,
            hidden_from_publishing: false,
            extends: None,
            overrides: BTreeSet::new(),
            code_syntax: BTreeMap::new(),
        });
        used_collections.insert(index);
        converted += 1;
    }
    for (index, (id, name, modes)) in plans.iter().enumerate() {
        if !used_collections.contains(&index) || doc.variables.collections.len() >= 256 {
            continue;
        }
        doc.variables.collections.push(Collection {
            id: id.clone(),
            name: name.clone(),
            modes: modes.iter().map(|(mode, name, _)| Mode { id: mode.clone(), name: name.clone() }).collect(),
            extends: None,
        });
    }
    let active_themes: Vec<&str> = value.get("$metadata").map(|m| list(m, "activeThemes").filter_map(Value::as_str).collect()).unwrap_or_default();
    for (group, themes) in &switching {
        let active = themes.iter().find(|t| active_themes.contains(&format!("{group}/{}", t.name).as_str()));
        if let Some(theme) = active {
            active_modes.insert(collection_id(group), format!("{}:{}", collection_id(group), theme.id));
        }
    }
    if converted > 0 {
        warn(
            warnings,
            format!(
                "Imported {converted} Penpot design tokens as native variables in {} collection(s); theme groups became modes",
                used_collections.len()
            ),
        );
    }
    if skipped > 0 {
        warn(
            warnings,
            format!(
                "{skipped} Penpot tokens (math expressions, composite typography/shadow values or mismatched references) were not converted; original JSON retained when possible"
            ),
        );
    }
    true
}

/// Import supported scalar tokens and retain their original source.
fn import_token_variables(
    z: &mut ZipArchive<Cursor<&[u8]>>,
    base: &str,
    doc: &mut Document,
    active_modes: &mut BTreeMap<String, String>,
    warnings: &mut Vec<String>,
    budget: &mut InflateBudget,
) -> Option<String> {
    let path = format!("{base}/tokens.json");
    z.index_for_name(&path)?;
    let value = match json(z, &path, budget) {
        Ok(v) => v,
        Err(_) => {
            warn(warnings, "Penpot tokens.json could not be read; tokens not imported".into());
            return None;
        }
    };
    import_composite_token_styles(&value, base, doc, warnings);
    let source = serde_json::to_string(&value).ok().filter(|s| s.len() <= 256 * 1024);
    if source.is_none() {
        warn(warnings, "Penpot tokens library exceeds 256 KiB metadata retention budget; only supported native variables will be saved".into());
    }
    let Some(sets) = value.get("sets").and_then(Value::as_object) else {
        if !import_themed_tokens(&value, base, doc, active_modes, warnings) {
            warn(warnings, "Unrecognized Penpot token sets format; original JSON retained when possible".into());
        }
        return source;
    };
    let mut count = 0usize;
    let mut skipped = 0usize;
    for (set_name, definition) in sets.iter().take(256) {
        if doc.variables.collections.len() >= 256 {
            warn(warnings, "Penpot variable collections capped at 256 across the document".into());
            break;
        }
        let Some(tokens) = definition.get("tokens") else {
            skipped += 1;
            continue;
        };
        let collection_id = format!("penpot:tokens:{base}:{set_name}");
        let mode_id = format!("{collection_id}:default");
        let mut flattened = Vec::new();
        flatten_tokens(tokens, "", &mut flattened, 0);
        let available = 8192usize.saturating_sub(doc.variables.variables.len());
        let flattened: Vec<_> = flattened.into_iter().take(available).collect();
        let index: BTreeMap<&str, &Value> = flattened.iter().map(|(name, token)| (name.as_str(), *token)).collect();
        let mut converted = 0usize;
        for (name, token) in &flattened {
            let token = *token;
            let converted_token = if let Some(target) = token_alias(token) {
                let mut visiting = BTreeSet::new();
                visiting.insert(name.as_str().to_owned());
                match (token_kind(token), resolved_token_kind(target, &index, &mut visiting, 0)) {
                    (Some(kind), Some(target_kind)) if kind == target_kind => Some((kind, VariableValue::Alias(format!("{collection_id}:{target}")))),
                    _ => None, // unresolved, cross-set, cyclic or mismatched types
                }
            } else {
                scalar_token(token)
            };
            if let Some((kind, item)) = converted_token {
                let mut values = BTreeMap::new();
                values.insert(mode_id.clone(), item);
                doc.variables.variables.push(Variable {
                    id: format!("{collection_id}:{name}"),
                    name: format!("{set_name}/{name}"),
                    collection: collection_id.clone(),
                    kind,
                    values,
                    description: string(token, "description")
                        .or_else(|| string(token, "$description"))
                        .unwrap_or_default()
                        .chars()
                        .take(1024)
                        .collect(),
                    scopes: None,
                    hidden_from_publishing: false,
                    extends: None,
                    overrides: BTreeSet::new(),
                    code_syntax: BTreeMap::new(),
                });
                count += 1;
                converted += 1;
            } else {
                skipped += 1;
            }
        }
        if converted > 0 {
            doc.variables.collections.push(Collection {
                id: collection_id,
                name: format!("Penpot/{set_name}"),
                modes: vec![Mode { id: mode_id, name: "Default".into() }],
                extends: None,
            });
        }
    }
    if sets.len() > 256 || doc.variables.variables.len() >= 8192 {
        warn(warnings, "Penpot token import capped at 256 sets and 8192 scalar variables".into());
    }
    if skipped > 0 {
        warn(
            warnings,
            format!("{skipped} unsupported or unresolved Penpot tokens were not converted to native variables; original JSON retained when possible"),
        );
    }
    if value.get("themes").is_some_and(|v| !v.is_null()) {
        warn(warnings, "Penpot themes and active set combinations are preserved as source metadata, not editable FigCraft modes".into());
    }
    if count > 0 {
        warn(
            warnings,
            format!(
                "Imported {count} Penpot scalar/alias design tokens as native variables; matching layer bindings will be linked when unambiguous"
            ),
        );
    }
    source
}

/// A source variant reference is meaningful even when its component is left
/// independent. Warn instead of marking such layers as linked editable variants.
fn report_design_system_limits(doc: &Document, warnings: &mut Vec<String>) {
    let in_set = |node: &figcraft_doc::Node| {
        matches!(node.kind, NodeKind::Component { .. })
            && node.parent.and_then(|p| doc.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::ComponentSet { .. }))
    };
    let variants = doc
        .nodes()
        .filter(|node| node.plugin_data.contains_key("penpot.variantId") || node.plugin_data.contains_key("penpot.source.variantProperties"))
        .filter(|node| !in_set(node))
        .count();
    if variants > 0 {
        warn(warnings, format!("{variants} Penpot variant layers are not inside a variant set; variant switching is unavailable for them"));
    }
    let missing = doc.nodes().filter(|node| node.plugin_data.get("penpot.componentLinkStatus").is_some_and(|status| status == "not-linked")).count();
    if missing > 0 {
        warn(warnings, format!("{missing} component copies have no native link; editing their masters will not update those copies"));
    }
}

/// File library components point at an existing page shape (`mainInstanceId`). Use this to turn
/// that original into a native FigCraft component. Copies are still independent layers until the
/// Penpot override/source graph can be translated safely (see docs).
fn component_masters(
    z: &mut ZipArchive<Cursor<&[u8]>>,
    base: &str,
    budget: &mut InflateBudget,
) -> (BTreeMap<String, String>, BTreeMap<String, String>, BTreeMap<String, String>) {
    let prefix = format!("{base}/components/");
    let paths = z
        .file_names()
        .filter(|name| name.strip_prefix(&prefix).is_some_and(|part| part.len() == 41 && part.ends_with(".json") && part.get(..36).is_some_and(uuid)))
        .take(10001)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut result = BTreeMap::new();
    let mut variant_names = BTreeMap::new();
    let mut sources = BTreeMap::new();
    if paths.len() > 10000 {
        return (result, variant_names, sources);
    }
    for path in paths {
        if let Ok(entry) = json(z, &path, budget)
            && let (Some(shape), Some(component)) =
                (string(&entry, "mainInstanceId").filter(|id| uuid(id)), string(&entry, "id").filter(|id| uuid(id)))
        {
            result.insert(shape.to_owned(), component.to_owned());
            if let Some(name) = variant_name(&entry) {
                variant_names.insert(shape.to_owned(), name);
            }
            let kept: serde_json::Map<String, Value> =
                entry.as_object().into_iter().flatten().filter(|(key, _)| key.as_str() != "objects").map(|(k, v)| (k.clone(), v.clone())).collect();
            if let Ok(encoded) = serde_json::to_string(&kept)
                && encoded.len() <= 16_384
            {
                sources.insert(shape.to_owned(), encoded);
            }
        }
    }
    (result, variant_names, sources)
}

/// Read a component variant label from its Penpot metadata.
fn variant_name(component: &Value) -> Option<String> {
    let clean = |s: &str| s.replace([',', '='], " ").trim().chars().take(200).collect::<String>();
    let pairs: Vec<String> = list(component, "variantProperties")
        .take(64)
        .filter_map(|p| Some((clean(string(p, "name")?), clean(string(p, "value").unwrap_or("")))))
        .filter(|(k, _)| !k.is_empty())
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    (!pairs.is_empty()).then(|| pairs.join(", "))
}

/// Apply saved variant names to imported component definitions.
fn name_variants(doc: &mut Document, file_id: &str, variant_names: &BTreeMap<String, String>) {
    let ids: Vec<NodeId> = doc
        .nodes()
        .filter(|n| matches!(n.kind, NodeKind::Component { .. }))
        .filter(|n| n.plugin_data.get("penpot.fileId").map(String::as_str) == Some(file_id))
        .filter(|n| n.parent.and_then(|p| doc.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::ComponentSet { .. })))
        .map(|n| n.id)
        .collect();
    for id in ids {
        let Some(node) = doc.node_mut(id) else { continue };
        let Some(name) = node.plugin_data.get("penpot.id").and_then(|pid| variant_names.get(pid)).cloned() else { continue };
        let original = std::mem::replace(&mut node.name, name);
        node.plugin_data.insert("penpot.originalName".into(), original);
    }
}

/// Append a warning while bounding the size of the import report.
fn warn(out: &mut Vec<String>, message: String) {
    if out.len() < 100 && !out.contains(&message) {
        out.push(message);
    }
}

/// Serialize Penpot's inline raw SVG without using an external process. The SVG importer
/// resolves curves, gradients and other supported elements into ordinary editable nodes.
/// Some exports contain a full XML string, others a `{tag, attrs, content}` tree.
/// Both representations are bounded before they enter the document.
fn svg_name(s: &str) -> bool {
    let mut bytes = s.bytes();
    bytes.next().is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, b'_' | b':'))
        && s.len() <= 100
        && bytes.all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b':' | b'.'))
}

/// Escape XML text or attribute content before rebuilding raw SVG.
fn svg_escape(value: &str, attribute: bool, out: &mut String) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            '\'' if attribute => out.push_str("&apos;"),
            c if c.is_control() && !matches!(c, '\n' | '\r' | '\t') => {}
            c => out.push(c),
        }
    }
}

/// Serialize a supported SVG node tree within the nesting and output budgets.
fn svg_tree(value: &Value, output: &mut String, depth: usize) -> Option<()> {
    if depth > 64 || output.len() > 128 * 1024 {
        return None;
    }
    match value {
        Value::String(s) => {
            svg_escape(s, false, output);
        }
        Value::Array(items) => {
            if items.len() > 8192 {
                return None;
            }
            for child in items {
                svg_tree(child, output, depth + 1)?;
            }
        }
        Value::Object(_) => {
            let tag = string(value, "tag")?;
            if !svg_name(tag) {
                return None;
            }
            output.push('<');
            output.push_str(tag);
            if let Some(attrs) = value.get("attrs").and_then(Value::as_object) {
                if attrs.len() > 256 {
                    return None;
                }
                for (key, val) in attrs {
                    if !svg_name(key) || key.to_ascii_lowercase().starts_with("on") {
                        continue;
                    }
                    let scalar = match val {
                        Value::String(v) => v.as_str(),
                        Value::Bool(_) | Value::Number(_) => {
                            // Render a bounded string; do not feed arbitrary JSON as an XML attribute.
                            output.push(' ');
                            output.push_str(key);
                            output.push_str("=\"");
                            svg_escape(&val.to_string(), true, output);
                            output.push('"');
                            continue;
                        }
                        _ => continue,
                    };
                    output.push(' ');
                    output.push_str(key);
                    output.push_str("=\"");
                    svg_escape(scalar, true, output);
                    output.push('"');
                }
            }
            output.push('>');
            if let Some(content) = value.get("content") {
                svg_tree(content, output, depth + 1)?;
            }
            output.push_str("</");
            output.push_str(tag);
            output.push('>');
        }
        Value::Null => {}
        _ => return None,
    }
    (output.len() <= 128 * 1024).then_some(())
}

/// Recover inline SVG markup or a serialized SVG tree from a raw shape.
fn raw_svg_xml(shape: &Value) -> Option<String> {
    let attrs = shape.get("svgAttrs").or_else(|| shape.get("svg-attrs"));
    let raw = attrs.and_then(|a| string(a, "raw")).or_else(|| shape.get("content").and_then(Value::as_str));
    if let Some(raw) = raw {
        let raw = raw.trim();
        if raw.len() > 128 * 1024 || raw.is_empty() || !raw.starts_with('<') {
            return None;
        }
        if raw.starts_with("<svg") || raw.starts_with("<?xml") {
            return Some(raw.to_owned());
        }
        let w = geometry(shape, "width", 1.0).max(1.0);
        let h = geometry(shape, "height", 1.0).max(1.0);
        return Some(format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\">{raw}</svg>"));
    }
    let content = shape.get("content")?;
    let mut xml = String::new();
    svg_tree(content, &mut xml, 0)?;
    if xml.starts_with("<svg ") || xml.starts_with("<svg>") {
        if xml.split('>').next().is_some_and(|header| header.contains("xmlns=")) {
            return Some(xml);
        }
        return Some(xml.replacen("<svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"", 1));
    }
    let w = geometry(shape, "width", 1.0).max(1.0);
    let h = geometry(shape, "height", 1.0).max(1.0);
    Some(format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\">{xml}</svg>"))
}

/// Parse a finite SVG length or percentage with a fallback for missing values.
fn svg_unit(raw: Option<&str>, default: f64) -> Option<f64> {
    let Some(raw) = raw else { return Some(default) };
    let trimmed = raw.trim();
    let n = match trimmed.strip_suffix('%') {
        Some(percent) => percent.trim().parse::<f64>().ok()? / 100.0,
        None => trimmed.parse::<f64>().ok()?,
    };
    (n.is_finite() && n.abs() <= 10_000.0).then_some(n)
}

/// Parse an SVG coordinate expressed in user-space units.
fn svg_user_unit(raw: Option<&str>) -> Option<f64> {
    let n = raw?.trim().trim_end_matches("px").parse::<f64>().ok()?;
    (n.is_finite() && n.abs() <= 1_000_000.0).then_some(n)
}

/// Find a property in a semicolon-separated inline CSS declaration.
fn css_property<'a>(style: &'a str, key: &str) -> Option<&'a str> {
    style.split(';').find_map(|decl| {
        let (k, v) = decl.split_once(':')?;
        (k.trim() == key).then(|| v.trim())
    })
}

/// Read an SVG gradient stop color and its opacity.
fn svg_stop_color(node: &Value) -> Option<Color> {
    let attrs = node.get("attrs")?;
    let style = attrs.get("style");
    let css = style.and_then(Value::as_str);
    let color = string(attrs, "stop-color")
        .or_else(|| string(attrs, "stopColor"))
        .or_else(|| style.and_then(|s| string(s, "stopColor")))
        .or_else(|| css.and_then(|s| css_property(s, "stop-color")))?;
    let c = Color::from_hex(color)?;
    let opacity = string(attrs, "stop-opacity")
        .or_else(|| string(attrs, "stopOpacity"))
        .or_else(|| style.and_then(|s| string(s, "stopOpacity")))
        .or_else(|| css.and_then(|s| css_property(s, "stop-opacity")))
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or_else(|| number(attrs, "stop-opacity", number(attrs, "stopOpacity", 1.0)));
    Some(c.with_alpha((f64::from(c.a) * opacity.clamp(0.0, 1.0)) as f32))
}

/// Parse a supported SVG transform list into a native affine matrix.
fn svg_transform_list(raw: &str) -> Option<Affine> {
    let mut result = Affine::IDENTITY;
    let mut rest = raw.trim();
    while !rest.is_empty() {
        let open = rest.find('(')?;
        let close = rest.find(')')?;
        let name = rest.get(..open)?.trim().trim_start_matches(',').trim();
        let args: Vec<f64> = rest
            .get(open + 1..close)?
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|a| !a.is_empty())
            .map(|a| a.parse::<f64>().ok())
            .collect::<Option<_>>()?;
        let step = match (name, args.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => Affine::new([*a, *b, *c, *d, *e, *f]),
            ("translate", [x]) => Affine::translate((*x, 0.0)),
            ("translate", [x, y]) => Affine::translate((*x, *y)),
            ("scale", [k]) => Affine::scale(*k),
            ("scale", [x, y]) => Affine::scale_non_uniform(*x, *y),
            ("rotate", [a]) => Affine::rotate(a.to_radians()),
            ("rotate", [a, x, y]) => Affine::translate((*x, *y)) * Affine::rotate(a.to_radians()) * Affine::translate((-x, -y)),
            ("skewX", [a]) => Affine::new([1.0, 0.0, a.to_radians().tan(), 1.0, 0.0, 0.0]),
            ("skewY", [a]) => Affine::new([1.0, a.to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
            _ => return None,
        };
        result *= step;
        rest = rest.get(close + 1..)?.trim();
    }
    result.as_coeffs().iter().all(|c| c.is_finite()).then_some(result)
}

/// Index gradient definitions in a bounded SVG tree traversal.
fn svg_gradient_nodes<'a>(value: &'a Value, key: Option<&str>, depth: usize, out: &mut BTreeMap<String, &'a Value>) {
    if depth > 32 || out.len() > 1024 {
        return;
    }
    if matches!(string(value, "tag"), Some("linearGradient" | "radialGradient"))
        && let Some(id) = value.get("attrs").and_then(|a| string(a, "id")).or(key)
    {
        out.entry(id.to_owned()).or_insert(value);
    }
    if let Some(items) = value.as_array() {
        for item in items.iter().take(1024) {
            svg_gradient_nodes(item, None, depth + 1, out);
        }
    } else if let Some(children) = value.get("content") {
        svg_gradient_nodes(children, None, depth + 1, out);
    } else if let Some(fields) = value.as_object() {
        for (k, child) in fields.iter().take(1024) {
            svg_gradient_nodes(child, Some(k), depth + 1, out);
        }
    }
}

/// Read a gradient reference from href or its legacy xlink spelling.
fn svg_href(attrs: &Value) -> Option<&str> {
    string(attrs, "xlinkHref").or_else(|| string(attrs, "href")).or_else(|| string(attrs, "xlink:href"))?.trim().strip_prefix('#')
}

/// Resolve inherited gradient attributes without following cyclic references.
fn svg_gradient_chain<'a>(defs: &BTreeMap<String, &'a Value>, id: &str) -> Option<(bool, serde_json::Map<String, Value>, &'a Value)> {
    let mut node = *defs.get(id)?;
    let radial = string(node, "tag")? == "radialGradient";
    let mut attrs = serde_json::Map::new();
    let mut stops = None;
    let mut visited = BTreeSet::new();
    for _ in 0..8 {
        let own = node.get("attrs")?;
        for (k, v) in own.as_object()? {
            attrs.entry(k.clone()).or_insert_with(|| v.clone());
        }
        if stops.is_none() && list(node, "content").any(|c| string(c, "tag") == Some("stop")) {
            stops = Some(node);
        }
        visited.insert(string(own, "id").unwrap_or(id).to_owned());
        match svg_href(own).filter(|h| !visited.contains(*h)).and_then(|h| defs.get(h)) {
            Some(next) => node = next,
            None => break,
        }
    }
    Some((radial, attrs, stops?))
}

/// Convert SVG stops and handles into a native gradient.
fn svg_gradient(radial: bool, attrs: &serde_json::Map<String, Value>, stops_node: &Value, to_unit: Option<Affine>) -> Option<Gradient> {
    let attrs = Value::Object(attrs.clone());
    let attrs = &attrs;
    if string(attrs, "spreadMethod").is_some_and(|s| s != "pad") {
        return None;
    }
    let user_space = match string(attrs, "gradientUnits") {
        None | Some("objectBoundingBox") => false,
        Some("userSpaceOnUse") => true,
        Some(_) => return None,
    };
    let coord = |key: &str, default: f64| if user_space { svg_user_unit(string(attrs, key)) } else { svg_unit(string(attrs, key), default) };
    let (from, to, width) = if radial {
        let cx = coord("cx", 0.5)?;
        let cy = coord("cy", 0.5)?;
        let r = coord("r", 0.5)?;
        let fx = if string(attrs, "fx").is_some() { coord("fx", cx)? } else { cx };
        let fy = if string(attrs, "fy").is_some() { coord("fy", cy)? } else { cy };
        // Native radial gradients cannot place an off-centre SVG focal point exactly.
        if r <= 0.0 || (fx - cx).abs() > 1e-6 || (fy - cy).abs() > 1e-6 {
            return None;
        }
        ((cx, cy), (cx + r, cy), (cx, cy + r))
    } else {
        let x1 = coord("x1", 0.0)?;
        let y1 = coord("y1", 0.0)?;
        let x2 = coord("x2", 1.0)?;
        let y2 = coord("y2", 0.0)?;
        ((x1, y1), (x2, y2), (x1 - (y2 - y1), y1 + x2 - x1))
    };
    let gradient_transform = match string(attrs, "gradientTransform") {
        Some(raw) => svg_transform_list(raw)?,
        None => Affine::IDENTITY,
    };
    let space = if user_space { to_unit? * gradient_transform } else { gradient_transform };
    let map = |p: (f64, f64)| {
        let q = space * figcraft_doc::geom::Point::new(p.0, p.1);
        (q.x.is_finite() && q.y.is_finite() && q.x.abs() <= 1e6 && q.y.abs() <= 1e6).then_some((q.x, q.y))
    };
    let (from, to, width) = (map(from)?, map(to)?, map(width)?);
    let mut stops = Vec::new();
    for stop in list(stops_node, "content").take(128) {
        if string(stop, "tag") != Some("stop") {
            continue;
        }
        let a = stop.get("attrs")?;
        let pos = svg_unit(string(a, "offset"), number(a, "offset", 0.0))?;
        stops.push(ColorStop { position: pos.clamp(0.0, 1.0) as f32, color: svg_stop_color(stop)? });
    }
    if stops.is_empty() {
        return None;
    }
    stops.sort_by(|a, b| a.position.total_cmp(&b.position));
    Some(Gradient { kind: if radial { GradientKind::Radial } else { GradientKind::Linear }, stops, from, to, width })
}

/// Read a named SVG gradient definition when its geometry is supported.
fn svg_gradient_definition(value: &Value) -> Option<(String, Gradient)> {
    let radial = match string(value, "tag")? {
        "linearGradient" => false,
        "radialGradient" => true,
        _ => return None,
    };
    let attrs = value.get("attrs")?;
    if svg_href(attrs).is_some() || string(attrs, "gradientUnits").is_some_and(|s| s != "objectBoundingBox") {
        return None;
    }
    let id = string(attrs, "id")?;
    if id.is_empty() || id.len() > 200 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':')) {
        return None;
    }
    Some((id.to_owned(), svg_gradient(radial, attrs.as_object()?, value, None)?))
}

/// Map saved SVG coordinates into the shape coordinate space.
fn svg_space_matrix(v: &Value) -> Option<Affine> {
    let m = match v.get("svgTransform").filter(|t| t.is_object()) {
        Some(t) => Affine::new([
            number(t, "a", 1.0),
            number(t, "b", 0.0),
            number(t, "c", 0.0),
            number(t, "d", 1.0),
            number(t, "e", 0.0),
            number(t, "f", 0.0),
        ]),
        None => Affine::IDENTITY,
    };
    m.as_coeffs().iter().all(|c| c.is_finite()).then_some(m)
}

/// Collect usable SVG gradients while tracking malformed definitions.
fn scan_svg_gradient_trees(value: &Value, depth: usize, count: &mut usize, out: &mut BTreeMap<String, Gradient>, bad: &mut BTreeSet<String>) {
    if depth > 32 || *count > 8192 {
        return;
    }
    *count += 1;
    if let Some((id, gradient)) = svg_gradient_definition(value)
        && out.insert(id.clone(), gradient).is_some()
    {
        bad.insert(id);
    }
    if let Some(items) = value.as_array() {
        for item in items.iter().take(1024) {
            scan_svg_gradient_trees(item, depth + 1, count, out, bad);
        }
    } else if let Some(children) = value.get("content") {
        scan_svg_gradient_trees(children, depth + 1, count, out, bad);
    } else if let Some(fields) = value.as_object() {
        for child in fields.values().take(1024) {
            scan_svg_gradient_trees(child, depth + 1, count, out, bad);
        }
    }
}

/// Resolve reusable SVG gradients from the imported shape metadata.
fn svg_gradients(shapes: &BTreeMap<String, Value>) -> BTreeMap<String, Gradient> {
    let mut result = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    let mut count = 0;
    for shape in shapes.values() {
        if count > 8192 {
            break;
        }
        if let Some(tree) = shape.get("content") {
            scan_svg_gradient_trees(tree, 0, &mut count, &mut result, &mut duplicates);
        }
        if let Some(defs) = shape.get("svgDefs") {
            scan_svg_gradient_trees(defs, 0, &mut count, &mut result, &mut duplicates);
        }
    }
    for id in duplicates {
        result.remove(&id);
    }
    result
}

/// Extract the local identifier of an SVG url paint reference.
fn svg_paint_ref(text: &str) -> Option<&str> {
    let id = text.trim().strip_prefix("url(#")?.strip_suffix(')')?.trim();
    (!id.is_empty() && id.len() <= 200).then_some(id)
}

/// What importing a page's layers shares from layer to layer: the archive and the page's shapes,
/// and the document, budgets and warnings they add to.
struct LayerImport<'a, 'b> {
    z: &'a mut ZipArchive<Cursor<&'b [u8]>>,
    base: &'a str,
    shapes: &'a BTreeMap<String, Value>,
    children_by_parent: &'a BTreeMap<String, Vec<String>>,
    svg_defs: &'a BTreeMap<String, Gradient>,
    masters: &'a BTreeMap<String, String>,
    doc: &'a mut Document,
    images: &'a mut BTreeMap<String, String>,
    image_bytes: &'a mut usize,
    inflate_budget: &'a mut InflateBudget,
    svg_bytes: &'a mut usize,
    warnings: &'a mut Vec<String>,
    seen: &'a mut BTreeSet<String>,
    count: &'a mut usize,
}

/// A Penpot layer's kind, with the box and outline of a decoded path (in the layer's own space).
fn layer_kind(
    v: &Value,
    typ: &str,
    is_master: bool,
    (x, y, w, h): (f64, f64, f64, f64),
    warnings: &mut Vec<String>,
) -> (NodeKind, Option<figcraft_doc::geom::Rect>, Option<BezPath>) {
    let mut path_box = None;
    let mut vector_source = None;
    let kind = match typ {
        "frame" if is_master => NodeKind::Component { description: String::new(), props: BTreeMap::new(), prop_order: Vec::new() },
        "frame" if v.get("isVariantContainer").and_then(Value::as_bool) == Some(true) => NodeKind::ComponentSet { description: String::new() },
        "frame" => NodeKind::Frame,
        "group" => NodeKind::Group,
        "bool" => NodeKind::BooleanOp {
            op: match string(v, "boolType") {
                Some("difference") => BoolOp::Subtract,
                Some("intersection") => BoolOp::Intersect,
                Some("exclude" | "exclusion") => BoolOp::Exclude,
                _ => BoolOp::Union,
            },
        },
        "rect" | "image" => NodeKind::Rectangle,
        // Expanded in the engine using the same native importer as File â†’ Import SVG.
        // This parsing crate must not depend on the higher-level SVG/render crates.
        "svg-raw" => NodeKind::Frame,
        "circle" | "ellipse" => NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 },
        "path" if v.get("content").and_then(Value::as_array).is_some_and(Vec::is_empty) => NodeKind::Vector { network: VectorNetwork::default() },
        "path" => match vector_bezpath(v, warnings) {
            Some(path) => {
                let local = shape_matrix(v, x, y, w, h).inverse() * Affine::translate((x, y)) * path;
                let bounds = local.bounding_box();
                let origin = if bounds.is_finite() {
                    path_box = Some(bounds);
                    bounds.origin()
                } else {
                    Default::default()
                };
                let local = Affine::translate(-origin.to_vec2()) * local;
                let network = VectorNetwork::from_path(&local);
                vector_source = Some(local);
                NodeKind::Vector { network }
            }
            None => {
                warn(warnings, "A Penpot path could not be decoded; kept as a frame".into());
                NodeKind::Frame
            }
        },
        "text" => NodeKind::Text(text_node(v)),
        other => {
            warn(warnings, format!("Unsupported Penpot layer type '{other}': imported as frame"));
            NodeKind::Frame
        }
    };
    (kind, path_box, vector_source)
}

/// A layer's stroke settings from its first Penpot stroke.
fn apply_stroke(node: &mut figcraft_doc::Node, stroke: &Value) {
    node.stroke.weight = number(stroke, "strokeWidth", 1.0).clamp(0.0, 10000.0);
    node.stroke.align = match string(stroke, "strokeAlignment") {
        Some("center") => StrokeAlign::Center,
        Some("outer") | Some("outside") => StrokeAlign::Outside,
        _ => StrokeAlign::Inside,
    };
    node.stroke.join = match string(stroke, "strokeJoin") {
        Some("round") => StrokeJoin::Round,
        Some("bevel") => StrokeJoin::Bevel,
        _ => StrokeJoin::Miter,
    };
    let cap = match string(stroke, "strokeCapStart") {
        Some("round") => StrokeCap::Round,
        Some("square") => StrokeCap::Square,
        Some("line-arrow") => StrokeCap::LineArrow,
        Some("triangle-arrow") => StrokeCap::TriangleArrow,
        _ => StrokeCap::None,
    };
    node.stroke.cap_start = cap;
    node.stroke.cap_end = match string(stroke, "strokeCapEnd") {
        Some("round") => StrokeCap::Round,
        Some("square") => StrokeCap::Square,
        Some("line-arrow") => StrokeCap::LineArrow,
        Some("triangle-arrow") => StrokeCap::TriangleArrow,
        _ => cap,
    };
    let w = node.stroke.weight;
    node.stroke.dashes = match string(stroke, "strokeStyle") {
        Some("dotted") => vec![0.0, w + 5.0],
        Some("dashed") => vec![w + 10.0, w + 10.0],
        Some("mixed") => vec![w + 5.0, w + 5.0, w + 1.0, w + 5.0],
        _ => Vec::new(),
    };
    if string(stroke, "strokeStyle") == Some("dotted") {
        let dot = |cap: StrokeCap| if matches!(cap, StrokeCap::LineArrow | StrokeCap::TriangleArrow) { cap } else { StrokeCap::Round };
        (node.stroke.cap_start, node.stroke.cap_end) = (dot(node.stroke.cap_start), dot(node.stroke.cap_end));
    }
}

/// A layer's visibility, lock, opacity, blend, clip, mask and constraints.
fn apply_appearance(node: &mut figcraft_doc::Node, v: &Value) {
    node.visible = v.get("hidden").and_then(Value::as_bool) != Some(true);
    node.locked = v.get("blocked").and_then(Value::as_bool) == Some(true);
    node.opacity = number(v, "opacity", 1.0).clamp(0.0, 1.0) as f32;
    node.blend = string(v, "blendMode").and_then(BlendMode::from_name).unwrap_or(BlendMode::Normal);
    node.clip = string(v, "type") == Some("frame") && v.get("showContent").and_then(Value::as_bool) != Some(true);
    node.is_mask = v.get("mask").and_then(Value::as_bool) == Some(true);
    node.mask_type = match string(v, "maskType") {
        Some("vector") => MaskType::Vector,
        Some("luminance") => MaskType::Luminance,
        _ => MaskType::Alpha,
    };
    node.constraints.horizontal = match string(v, "constraintsH") {
        Some("right") => HConstraint::Right,
        Some("leftright") | Some("left-right") => HConstraint::LeftRight,
        Some("center") => HConstraint::Center,
        Some("scale") => HConstraint::Scale,
        _ => HConstraint::Left,
    };
    node.constraints.vertical = match string(v, "constraintsV") {
        Some("bottom") => VConstraint::Bottom,
        Some("topbottom") | Some("top-bottom") => VConstraint::TopBottom,
        Some("center") => VConstraint::Center,
        Some("scale") => VConstraint::Scale,
        _ => VConstraint::Top,
    };
}

/// An `svg-raw` layer's source tree and markup, kept for the engine to expand (bounded by
/// `svg_bytes`, the archive's raw SVG budget).
fn keep_raw_svg(node: &mut figcraft_doc::Node, v: &Value, name: &str, svg_bytes: &mut usize, warnings: &mut Vec<String>) {
    // If future SVG support improves, keep the original tree (when small)
    // alongside the reconstructed markup for faithful re-import.
    preserve_source(node, v, &["content", "svgAttrs", "svg-attrs"]);
    if let Some(xml) = raw_svg_xml(v) {
        // Avoid unbounded retained markup in huge archives or projects.
        if xml.len() <= (8 * 1024 * 1024usize).saturating_sub(*svg_bytes) {
            *svg_bytes += xml.len();
            node.plugin_data.insert("penpot.svgRawXml".into(), xml);
            node.plugin_data.insert("penpot.svgRawStatus".into(), "pending".into());
        } else {
            warn(warnings, format!("SVG layer '{name}' exceeds the 8 MiB raw SVG import budget; kept as frame"));
        }
    } else {
        warn(warnings, format!("SVG layer '{name}' has no safely readable SVG markup; kept as frame"));
    }
}

/// A text layer's font references, kept as the source names them.
fn keep_font_refs(node: &mut figcraft_doc::Node, v: &Value) {
    let references = referenced_google_fonts(v);
    if !references.is_empty() {
        node.plugin_data.insert("penpot.googleFontRefs".into(), serde_json::to_string(&references).unwrap_or_default());
    }
    // A `custom-*` identifier refers to Penpot's separately managed
    // font service. It is not a URL and does not imply that the binary
    // is contained in this `.penpot` archive. Preserve the exact id so
    // the desktop importer can report or resolve it without guessing.
    let custom = referenced_custom_fonts(v);
    if !custom.is_empty() {
        node.plugin_data.insert("penpot.customFontRefs".into(), serde_json::to_string(&custom).unwrap_or_default());
    }
}

/// The Penpot source fields a layer keeps for a faithful re-export (see [`preserve_source`]).
const PRESERVED_LAYER_KEYS: &[&str] = &[
    "layout",
    "layoutMode",
    "layoutFlexDir",
    "layoutWrapType",
    "layoutGap",
    "layoutPadding",
    "layoutJustifyContent",
    "layoutAlignItems",
    "layoutAlignContent",
    "layoutGridColumns",
    "layoutGridRows",
    "layoutGridCells",
    "layoutItemMargin",
    "layoutItemAlignSelf",
    "layoutItemAbsolute",
    "layoutItemHSizing",
    "layoutItemVSizing",
    "touched",
    "appliedTokens",
    "interactions",
    "variantProperties",
    "pluginData",
    "grids",
    "guides",
    "maskType",
    "svgAttrs",
    "svgViewbox",
    "svgDefs",
    "layoutGapType",
    "layoutPaddingType",
    "layoutJustifyItems",
    "layoutGridDir",
    "componentRoot",
    "mainInstance",
    "layoutItemZIndex",
    "exports",
    "hideInViewer",
    "strokes",
];

/// The Penpot ids a layer keeps: its own, its file's and its component's.
fn keep_ids(node: &mut figcraft_doc::Node, v: &Value, id: &str, base: &str, master: Option<&String>) {
    node.plugin_data.insert("penpot.id".into(), id.to_owned());
    if let Some(file_id) = base.strip_prefix("files/") {
        node.plugin_data.insert("penpot.fileId".into(), file_id.to_owned());
    }
    if let Some(component) = master {
        node.plugin_data.insert("penpot.componentMasterId".into(), component.clone());
    }
    if v.get("componentId").is_some() && master.is_none() {
        node.plugin_data.insert("penpot.componentLinkStatus".into(), "not-linked".into());
    }
}

/// The Penpot source fields a layer keeps: the layout and other settings FigCraft can't hold, and
/// its component references.
fn keep_source(node: &mut figcraft_doc::Node, v: &Value, typ: &str) {
    // Mark unsupported properties on their exact source nodes. Layout changes may
    // otherwise silently discard these Penpot-specific rules on subsequent edits.
    if string(v, "layoutItemAlignSelf")
        .is_some_and(|a| !matches!(a, "auto" | "start" | "flex-start" | "center" | "end" | "flex-end" | "stretch" | "baseline"))
    {
        node.plugin_data.insert("penpot.unsupported.alignSelf".into(), "true".into());
    }
    preserve_source(node, v, PRESERVED_LAYER_KEYS);
    if typ == "text" {
        preserve_source_within(node, v, &["content", "positionData"], 256 * 1024);
    }
    for key in ["componentId", "componentFile", "shapeRef", "variantId"] {
        if let Some(value) = string(v, key)
            && value.len() <= 1024
        {
            node.plugin_data.insert(format!("penpot.{key}"), value.to_owned());
        }
    }
}

/// A text layer's fills: a paint all its runs share moves to the layer, and a text layer with no
/// fill gets black.
fn settle_text_fills(node: &mut figcraft_doc::Node, typ: &str) {
    if let NodeKind::Text(text) = &mut node.kind
        && let Some(paints) = uniform_text_fills(text)
    {
        node.fills = paints;
        text.style.fills = None;
        for run in &mut text.runs {
            run.style.fills = None;
        }
    }
    if typ == "text" && node.fills.is_empty() {
        node.fills.push(Paint::solid(Color::BLACK));
    }
}

impl LayerImport<'_, '_> {
    /// Penpot paints of `v`'s `key` (fills or strokes).
    fn layer_paints(&mut self, v: &Value, key: &str) -> Vec<Paint> {
        paints(self.z, self.base, v, key, self.doc, self.images, self.image_bytes, self.inflate_budget, self.warnings)
    }

    /// A shape's fills from its SVG attributes (a style or attribute fill, or one inherited from its
    /// SVG ancestors), and a paint-server reference resolved to a gradient, when it has no fills of
    /// its own.
    fn svg_fills(&mut self, v: &Value, typ: &str, abs: Affine, (x, y, w, h): (f64, f64, f64, f64), mut fills: Vec<Paint>) -> Vec<Paint> {
        let svg = v.get("svgAttrs");
        let style_fill = svg.and_then(|a| a.get("style")).and_then(|s| string(s, "fill"));
        let attr_fill = svg.and_then(|a| string(a, "fill"));
        let source = match style_fill {
            Some(fill) if Color::from_hex(fill).is_some() => Some(serde_json::json!({"fills": [{"fillColor": fill}]})),
            Some(_) => None,
            None if list(v, "fills").next().is_some() => None,
            None => Some(match attr_fill {
                Some(fill) => serde_json::json!({"fills": [{"fillColor": fill}]}),
                None => inherited_fill(self.shapes, v),
            }),
        };
        if let Some(source) = source {
            fills = self.layer_paints(&source, "fills");
        }
        let Some(reference) = style_fill.or(attr_fill).and_then(svg_paint_ref).filter(|_| fills.is_empty()) else { return fills };
        let mut local = BTreeMap::new();
        if let Some(defs) = v.get("svgDefs") {
            svg_gradient_nodes(defs, None, 0, &mut local);
        }
        let resolved = if local.contains_key(reference) {
            let to_local = if typ == "path" { abs.inverse() } else { Affine::translate((-x, -y)) };
            let to_unit = svg_space_matrix(v).map(|m| Affine::scale_non_uniform(1.0 / w, 1.0 / h) * to_local * m);
            svg_gradient_chain(&local, reference).and_then(|(radial, attrs, stops)| svg_gradient(radial, &attrs, stops, to_unit))
        } else {
            self.svg_defs.get(reference).cloned()
        };
        match resolved {
            Some(g) => vec![Paint::gradient(g)],
            None => {
                warn(
                    self.warnings,
                    format!("Penpot SVG paint-server url(#{reference}) cannot be represented as a native gradient; original SVG attributes retained"),
                );
                fills
            }
        }
    }

    /// A layer's fills: its own, a stand-alone image's from its metadata, or its SVG attributes'.
    fn layer_fills(&mut self, v: &Value, typ: &str, abs: Affine, geometry: (f64, f64, f64, f64)) -> Vec<Paint> {
        let mut fills = self.layer_paints(v, "fills");
        // Some Penpot versions keep a stand-alone image's reference in `metadata`, not `fills`.
        if typ == "image"
            && let Some(metadata) = v.get("metadata")
        {
            let mut stretched = metadata.clone();
            if let Some(obj) = stretched.as_object_mut() {
                obj.insert("keepAspectRatio".into(), Value::Bool(false));
            }
            let mut all = vec![serde_json::json!({"fillImage": stretched, "fillOpacity": 1.0})];
            all.extend(list(v, "fills").cloned());
            let temporary = serde_json::json!({ "fills": all });
            fills = self.layer_paints(&temporary, "fills");
        }
        if matches!(typ, "rect" | "circle" | "path" | "bool") {
            fills = self.svg_fills(v, typ, abs, geometry, fills);
        }
        fills
    }

    /// Penpot layer `id` and its subtree, under `parent` (whose absolute transform is
    /// `parent_abs`), in grid `cell` of its parent when it has one.
    fn add_layer(&mut self, id: &str, parent: NodeId, parent_abs: Affine, cell: Option<&Value>, depth: usize) -> Result<(), FormatError> {
        if depth >= MAX_DEPTH {
            warn(self.warnings, "Penpot layer nesting deeper than 128 was omitted".into());
            return Ok(());
        }
        if !self.seen.insert(id.to_owned()) {
            return Ok(());
        }
        let shapes = self.shapes;
        let Some(v) = shapes.get(id) else {
            warn(self.warnings, format!("Missing layer {id}"));
            return Ok(());
        };
        *self.count += 1;
        if *self.count > MAX_LAYERS {
            return Err(corrupt("too many layers"));
        }
        let name = string(v, "name").unwrap_or("Layer").chars().take(512).collect::<String>();
        let x = geometry(v, "x", 0.0);
        let y = geometry(v, "y", 0.0);
        let w = geometry(v, "width", 1.0).clamp(0.01, 1_000_000.0);
        let h = geometry(v, "height", 1.0).clamp(0.01, 1_000_000.0);
        let typ = string(v, "type").unwrap_or("");
        let (kind, path_box, vector_source) = layer_kind(v, typ, self.masters.contains_key(id), (x, y, w, h), self.warnings);
        let abs = if kind.bounds_follow_children() { Affine::translate((x, y)) } else { shape_matrix(v, x, y, w, h) };
        let (abs, w, h) = match path_box {
            Some(b) => (abs * Affine::translate(b.origin().to_vec2()), b.width().max(0.01), b.height().max(0.01)),
            None => (abs, w, h),
        };
        let created = self.doc.create(parent, kind, &name, 0.0, 0.0, w, h).map_err(|e| corrupt(format!("layer '{name}': {e}")))?;
        let mut fills = self.layer_fills(v, typ, abs, (x, y, w, h));
        let mut strokes = self.layer_paints(v, "strokes");
        // Media metadata and image reads are optional, but exhausting the shared
        // decompression budget is never optional: abort rather than continue importing.
        self.inflate_budget.check()?;
        if typ == "svg-raw" {
            fills.clear();
            strokes.clear();
        }
        if !fills.is_empty()
            && let Some(path) = vector_source
            && let Some(node) = self.doc.node_mut(created)
        {
            node.kind = NodeKind::Vector { network: filled_network(&path) };
        }
        let svg = v.get("svgAttrs");
        let fill_rule = svg.and_then(|a| a.get("style")).and_then(|s| string(s, "fillRule")).or_else(|| svg.and_then(|a| string(a, "fillRule")));
        if fill_rule == Some("evenodd")
            && let Some(NodeKind::Vector { network }) = self.doc.node_mut(created).map(|n| &mut n.kind)
        {
            for region in &mut network.regions {
                region.winding = figcraft_doc::WindingRule::EvenOdd;
            }
        }
        if let Some(node) = self.doc.node_mut(created) {
            // Unlike Node::new's design-tool defaults, absence of a Penpot fill means no fill.
            node.fills = fills;
            node.strokes = strokes;
            apply_appearance(node, v);
            node.effects = effects(v, self.warnings);
            if node.kind.is_frame_like() {
                node.grids = display_grids(v, self.warnings);
                node.guides = penpot_guides(v.get("guides"));
            }
            if typ == "svg-raw" {
                keep_raw_svg(node, v, &name, self.svg_bytes, self.warnings);
            }
            if typ == "text" {
                keep_font_refs(node, v);
            }
            keep_ids(node, v, id, self.base, self.masters.get(id));
            apply_child_layout(v, cell, node);
            // The prototype links are kept until every page is in the document, when the
            // Penpot shape ids they name can be resolved to FigCraft node ids.
            prototype::keep_interaction_source(node, v, self.warnings);
            keep_source(node, v, typ);
            node.constrain_proportions = v.get("proportionLock").and_then(Value::as_bool) == Some(true);
            settle_text_fills(node, typ);
            let radius = number(v, "rx", 0.0).max(0.0);
            node.corners.radii = [
                number(v, "r1", radius).max(0.0),
                number(v, "r2", radius).max(0.0),
                number(v, "r3", radius).max(0.0),
                number(v, "r4", radius).max(0.0),
            ];
            node.transform = parent_abs.inverse() * abs;
            if let Some(stroke) = list(v, "strokes").next() {
                apply_stroke(node, stroke);
            }
        }
        self.add_children(v, id, created, abs, depth)?;
        finish_layout(self.doc, created, v, self.warnings);
        finish_masked_group(self.doc, created, v, self.warnings);
        Ok(())
    }

    /// Layer `created`'s auto layout and its children: those `v` lists, in its stacking order, then
    /// any whose parent id names it but that the list left out.
    fn add_children(&mut self, v: &Value, id: &str, created: NodeId, abs: Affine, depth: usize) -> Result<(), FormatError> {
        if v.get("layout").is_some() || v.get("layoutMode").is_some() {
            let layout = layout_props(v, self.warnings);
            if let Some(node) = self.doc.node_mut(created) {
                if !node.kind.is_frame_like() {
                    warn(self.warnings, "Layout on non-frame-like Penpot layer retained as metadata".into());
                } else {
                    node.layout = layout;
                }
            }
        }
        let mut children: Vec<String> = list(v, "shapes").filter_map(Value::as_str).map(str::to_owned).collect();
        if let Some(layout) =
            self.doc.node_mut(created).and_then(|n| n.layout.as_mut()).filter(|l| matches!(l.mode, LayoutMode::Horizontal | LayoutMode::Vertical))
        {
            layout.reverse_z = true;
            children.reverse();
        }
        let cells = grid_cells(v);
        for child in children {
            self.add_layer(&child, created, abs, cells.get(child.as_str()).copied(), depth + 1)?;
        }
        // `shapes` may be incomplete in a damaged or partially exported document. Recover
        // genuine descendants through their declared parent without reordering listed layers.
        let children_by_parent = self.children_by_parent;
        if let Some(unlinked) = children_by_parent.get(id) {
            for child in unlinked {
                if !self.seen.contains(child) {
                    warn(self.warnings, "Penpot group omitted a child from its stacking list; recovered it by parentId".into());
                    self.add_layer(child, created, abs, cells.get(child.as_str()).copied(), depth + 1)?;
                }
            }
        }
        Ok(())
    }
}

/// Exact font-provider references supplied by Penpot's text content tree.
/// Preserve these independently of display names; a name alone does not prove a
/// font comes from Google Fonts, and an archive may contain mixed-style runs.
fn referenced_google_fonts(source: &Value) -> Vec<Value> {
    referenced_font_refs(source, "gfont-")
}

/// Collect custom font references for diagnostics without resolving them remotely.
fn referenced_custom_fonts(source: &Value) -> Vec<Value> {
    referenced_font_refs(source, "custom-")
}

/// Collect bounded provider references and their requested font variants.
fn referenced_font_refs(source: &Value, provider_prefix: &str) -> Vec<Value> {
    #[derive(Clone)]
    struct FontContext {
        id: Option<String>,
        family: Option<String>,
        weight: u16,
        italic: bool,
    }
    fn weight(v: &Value) -> Option<u16> {
        let num = v.as_u64().and_then(|v| u16::try_from(v).ok()).or_else(|| v.as_str().and_then(|s| s.parse::<u16>().ok())).or_else(|| {
            match v.as_str()?.to_ascii_lowercase().as_str() {
                "thin" => Some(100),
                "extralight" | "ultralight" => Some(200),
                "light" => Some(300),
                "regular" | "normal" => Some(400),
                "medium" => Some(500),
                "semibold" | "demibold" => Some(600),
                "bold" => Some(700),
                "extrabold" | "ultrabold" => Some(800),
                "black" | "heavy" => Some(900),
                _ => None,
            }
        });
        num.filter(|n| (1..=1000).contains(n))
    }
    fn visit(
        node: &Value,
        out: &mut BTreeMap<(String, String), BTreeSet<(u16, bool)>>,
        depth: usize,
        inherited: &FontContext,
        provider_prefix: &str,
    ) {
        if depth > 32 || out.len() >= 16 {
            return;
        }
        let mut current = inherited.clone();
        if let Some(id) = string(node, "fontId") {
            current.id = Some(id.to_owned());
        }
        if let Some(family) = string(node, "fontFamily") {
            current.family = Some(family.to_owned());
        }
        if let Some(w) = node.get("fontWeight").and_then(weight) {
            current.weight = w;
        }
        if let Some(style) = string(node, "fontStyle") {
            current.italic = style.eq_ignore_ascii_case("italic") || style.eq_ignore_ascii_case("oblique");
        }
        if let Some(variant) = string(node, "fontVariantId") {
            let variant = variant.to_ascii_lowercase();
            if node.get("fontWeight").is_none() {
                let digits: String = variant.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(weight) = digits.parse::<u16>()
                    && (1..=1000).contains(&weight)
                {
                    current.weight = weight;
                }
            }
            current.italic = variant.contains("italic") || variant.contains("oblique");
        }
        if let (Some(id), Some(family)) = (&current.id, &current.family)
            && id.starts_with(provider_prefix)
            && id.len() <= 128
            && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            && !family.is_empty()
            && family.len() <= 128
        {
            let variants = out.entry((id.clone(), family.clone())).or_default();
            if variants.len() < 8 {
                variants.insert((current.weight, current.italic));
            }
        }
        // Parent paragraphs and child spans may declare different portions of the
        // font reference. Walk both the shape's content root and its styled runs.
        if let Some(content) = node.get("content") {
            visit(content, out, depth + 1, &current, provider_prefix);
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children.iter().take(1024) {
                visit(child, out, depth + 1, &current, provider_prefix);
            }
        }
    }
    let mut refs = BTreeMap::new();
    visit(source, &mut refs, 0, &FontContext { id: None, family: None, weight: 400, italic: false }, provider_prefix);
    refs.into_iter()
        .map(|((id, family), variants)| {
            serde_json::json!({
                "id": id,
                "family": family,
                "variants": variants.into_iter().map(|(weight, italic)| serde_json::json!({"weight": weight, "italic": italic})).collect::<Vec<_>>()
            })
        })
        .collect()
}

/// Return common text paints only when every styled run agrees.
fn uniform_text_fills(text: &TextData) -> Option<Vec<Paint>> {
    let base = text.style.fills.as_ref();
    let mut runs: Vec<(usize, usize)> = text.runs.iter().map(|r| (r.start, r.end)).collect();
    runs.sort_unstable();
    let covered = runs.iter().try_fold(0, |at, &(start, end)| (start <= at).then_some(at.max(end)));
    let covers_all = covered.is_some_and(|end| end >= text.characters.chars().count());
    let mut colors: Vec<Option<&Vec<Paint>>> = text.runs.iter().map(|r| r.style.fills.as_ref().or(base)).collect();
    if !covers_all {
        colors.push(base);
    }
    let first = (*colors.first()?)?;
    colors.iter().all(|c| *c == Some(first)).then(|| first.clone())
}

/// Apply supported Penpot text properties over an inherited native style.
fn text_style(v: &Value, base: &TextStyle) -> TextStyle {
    let mut style = base.clone();
    if let Some(family) = string(v, "fontFamily").or_else(|| string(v, "fontId")) {
        style.font_family = if family == "sourcesanspro" { "Source Sans Pro".into() } else { family.chars().take(200).collect() };
    }
    let size = number(v, "fontSize", style.font_size);
    if size > 0.0 && size <= 1000.0 {
        style.font_size = size;
    }
    // Penpot can specify the face only by `fontVariantId` (e.g. 700italic),
    // including in a leaf that inherited its family from a parent paragraph.
    let variant = string(v, "fontVariantId").unwrap_or("").to_ascii_lowercase();
    let variant_weight: String = variant.chars().take_while(|c| c.is_ascii_digit()).collect();
    let weight = v
        .get("fontWeight")
        .and_then(|w| w.as_str().map(str::to_owned).or_else(|| w.as_u64().map(|n| n.to_string())))
        .or_else(|| (!variant_weight.is_empty()).then_some(variant_weight));
    if let Some(weight) = weight {
        style.font_style = match weight.to_ascii_lowercase().as_str() {
            "100" | "thin" => "Thin",
            "200" | "extralight" | "extra-light" => "Extra Light",
            "300" | "light" => "Light",
            "500" | "medium" => "Medium",
            "600" | "semibold" | "semi-bold" => "Semi Bold",
            "700" | "bold" => "Bold",
            "800" | "extrabold" | "extra-bold" => "Extra Bold",
            "900" | "black" => "Black",
            _ => "Regular",
        }
        .into();
    }
    let italic = variant.contains("italic")
        || variant.contains("oblique")
        || string(v, "fontStyle").is_some_and(|v| v.eq_ignore_ascii_case("italic") || v.eq_ignore_ascii_case("oblique"));
    if italic && !style.font_style.contains("Italic") {
        style.font_style.push_str(" Italic");
    }
    match string(v, "textDecoration") {
        Some("underline") => style.decoration = TextDecoration::Underline,
        Some("line-through") | Some("strikethrough") => style.decoration = TextDecoration::Strikethrough,
        _ => {}
    }
    style.case = match string(v, "textTransform") {
        Some("uppercase") => TextCase::Upper,
        Some("lowercase") => TextCase::Lower,
        Some("capitalize") => TextCase::Title,
        _ => style.case,
    };
    if v.get("letterSpacing").is_some() {
        style.letter_spacing = LetterSpacing::Pixels(number(v, "letterSpacing", 0.0));
    }
    if v.get("lineHeight").is_some() {
        let line = number(v, "lineHeight", 1.2);
        if line > 0.0 {
            style.line_height = LineHeight::Percent(line * 100.0);
        }
    }
    let mut paints: Vec<Paint> = list(v, "fills")
        .filter_map(|fill| {
            let mut p = match fill.get("fillColorGradient").and_then(gradient) {
                Some(g) => Paint::gradient(g),
                None => Paint::solid(string(fill, "fillColor").and_then(Color::from_hex)?),
            };
            p.opacity *= number(fill, "fillOpacity", 1.0).clamp(0.0, 1.0) as f32;
            Some(p)
        })
        .take(32)
        .collect();
    paints.reverse();
    if !paints.is_empty() {
        style.fills = Some(paints);
    }
    style
}

/// Penpot nests paragraphs, lines and character runs. Keep runs at Unicode character offsets;
/// bytes would truncate multibyte characters when the user edits the resulting FigCraft text.
fn text_node(v: &Value) -> TextData {
    let mut fragments: Vec<(String, Value)> = Vec::new();
    fn own_style(v: &Value, inherited: &serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
        // Text formatting can be declared on root, paragraph, span, or leaf.
        // Inherit explicit properties down the tree rather than losing them if
        // the actual text run only contains the `text` field.
        let mut style = inherited.clone();
        for key in [
            "fontFamily",
            "fontId",
            "fontVariantId",
            "fontSize",
            "fontWeight",
            "fontStyle",
            "textDecoration",
            "textTransform",
            "letterSpacing",
            "lineHeight",
            "fills",
            "textAlign",
        ] {
            if let Some(value) = v.get(key) {
                style.insert(key.into(), value.clone());
            }
        }
        if v.get("fills").is_none() && (v.get("fillColor").is_some() || v.get("fillColorGradient").is_some()) {
            let legacy: serde_json::Map<String, Value> =
                ["fillColor", "fillOpacity", "fillColorGradient"].iter().filter_map(|k| Some(((*k).to_owned(), v.get(*k)?.clone()))).collect();
            style.insert("fills".into(), Value::Array(vec![Value::Object(legacy)]));
        }
        style
    }
    fn walk(v: &Value, result: &mut Vec<(String, Value)>, depth: usize, inherited: &serde_json::Map<String, Value>) {
        if depth >= 64 || result.len() >= 10_000 {
            return;
        }
        let style = own_style(v, inherited);
        if let Some(t) = string(v, "text") {
            result.push((t.chars().take(100_000).collect(), Value::Object(style.clone())));
        }
        if string(v, "type") == Some("paragraph-set") {
            let mut previous = None;
            for child in list(v, "children") {
                if let Some(ended) = previous.take() {
                    result.push(("\n".into(), Value::Object(ended)));
                }
                walk(child, result, depth + 1, &style);
                previous = Some(own_style(child, &style));
            }
        } else {
            for child in list(v, "children") {
                walk(child, result, depth + 1, &style);
            }
        }
    }
    let mut initial = serde_json::Map::new();
    if let Some(root) = v.get("content") {
        for key in [
            "fontFamily",
            "fontId",
            "fontVariantId",
            "fontSize",
            "fontWeight",
            "fontStyle",
            "textDecoration",
            "textTransform",
            "letterSpacing",
            "lineHeight",
            "fills",
            "textAlign",
        ] {
            if let Some(value) = v.get(key) {
                initial.insert(key.into(), value.clone());
            }
        }
        for key in [
            "fontFamily",
            "fontId",
            "fontVariantId",
            "fontSize",
            "fontWeight",
            "fontStyle",
            "textDecoration",
            "textTransform",
            "letterSpacing",
            "lineHeight",
            "fills",
            "textAlign",
        ] {
            if let Some(value) = root.get(key) {
                initial.insert(key.into(), value.clone());
            }
        }
        walk(root, &mut fragments, 0, &initial);
    }
    let mut result = TextData::new(String::new());
    // The first run may contain formatting that applies *only* to that run.
    // Deriving the base style from it would leak e.g. its Bold weight into
    // unrelated siblings. Only the common root style becomes the base.
    result.style = text_style(&Value::Object(initial), &result.style);
    let root_style = fragments.first().map(|(_, s)| s.clone()).unwrap_or_else(|| v.clone());
    let mut current = 0usize;
    for (piece, props) in fragments {
        let size = piece.chars().count();
        if size == 0 || current.saturating_add(size) > 100_000 {
            continue;
        }
        let st = text_style(&props, &result.style);
        if st != result.style {
            result.runs.push(StyleRun { start: current, end: current + size, style: st });
        }
        result.characters.push_str(&piece);
        current += size;
    }
    result.align_h = match string(&root_style, "textAlign") {
        Some("center") => TextAlignH::Center,
        Some("right") => TextAlignH::Right,
        Some("justify") => TextAlignH::Justified,
        _ => TextAlignH::Left,
    };
    result.auto_resize = match string(v, "growType") {
        Some("fixed") => TextAutoResize::None,
        Some("auto-height") => TextAutoResize::Height,
        _ => TextAutoResize::WidthAndHeight,
    };
    result
}

/// Approximate SVG/Penpot elliptical endpoint arcs with at most four editable cubic
/// Bézier segments. These are *native* FigCraft vector handles, not an SVG image.
/// Uses the W3C SVG endpoint-to-centre conversion and keeps all intermediate values finite.
fn arc_to_cubics(path: &mut BezPath, from: (f64, f64), to: (f64, f64), rx: f64, ry: f64, rotation_degrees: f64, large: bool, sweep: bool) -> bool {
    use std::f64::consts::{FRAC_PI_2, PI};
    if ![from.0, from.1, to.0, to.1, rx, ry, rotation_degrees].iter().all(|n| n.is_finite()) {
        return false;
    }
    let (mut rx, mut ry) = (rx.abs().min(1e6), ry.abs().min(1e6));
    if rx < 1e-9 || ry < 1e-9 {
        path.line_to(to);
        return true;
    }
    if (from.0 - to.0).abs() < 1e-9 && (from.1 - to.1).abs() < 1e-9 {
        // Identical endpoints are a zero-length arc under the SVG spec.
        return true;
    }
    let phi = rotation_degrees.rem_euclid(360.0).to_radians();
    let (sin, cos) = phi.sin_cos();
    let (dx, dy) = ((from.0 - to.0) * 0.5, (from.1 - to.1) * 0.5);
    let (xp, yp) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    let lambda = (xp / rx).powi(2) + (yp / ry).powi(2);
    if !lambda.is_finite() {
        return false;
    }
    if lambda > 1.0 {
        let scale = lambda.sqrt();
        rx *= scale;
        ry *= scale;
    }
    let a = rx * rx * ry * ry;
    let b = rx * rx * yp * yp + ry * ry * xp * xp;
    if b <= 1e-24 || !a.is_finite() || !b.is_finite() {
        return false;
    }
    let sign = if large == sweep { -1.0 } else { 1.0 };
    let k = sign * ((a - b).max(0.0) / b).sqrt();
    let (cxp, cyp) = (k * rx * yp / ry, -k * ry * xp / rx);
    let cx = (from.0 + to.0) * 0.5 + cos * cxp - sin * cyp;
    let cy = (from.1 + to.1) * 0.5 + sin * cxp + cos * cyp;
    let (sx, sy) = ((xp - cxp) / rx, (yp - cyp) / ry);
    let (ex, ey) = ((-xp - cxp) / rx, (-yp - cyp) / ry);
    let start = sy.atan2(sx);
    let mut delta = (sx * ey - sy * ex).atan2(sx * ex + sy * ey);
    if sweep && delta < 0.0 {
        delta += 2.0 * PI;
    }
    if !sweep && delta > 0.0 {
        delta -= 2.0 * PI;
    }
    let segments = (delta.abs() / FRAC_PI_2).ceil().max(1.0) as usize;
    if segments > 4 || ![cx, cy, start, delta].iter().all(|n| n.is_finite()) {
        return false;
    }
    let step = delta / segments as f64;
    let point = |angle: f64| {
        let (s, c) = angle.sin_cos();
        (cx + rx * cos * c - ry * sin * s, cy + rx * sin * c + ry * cos * s)
    };
    let tangent = |angle: f64| {
        let (s, c) = angle.sin_cos();
        (-rx * cos * s - ry * sin * c, -rx * sin * s + ry * cos * c)
    };
    let mut last = from;
    for i in 0..segments {
        let a0 = start + step * i as f64;
        let a1 = a0 + step;
        let k = (4.0 / 3.0) * (step / 4.0).tan();
        let t0 = tangent(a0);
        let t1 = tangent(a1);
        let end = if i + 1 == segments { to } else { point(a1) };
        let c1 = (last.0 + k * t0.0, last.1 + k * t0.1);
        let c2 = (end.0 - k * t1.0, end.1 - k * t1.1);
        if ![c1.0, c1.1, c2.0, c2.1, end.0, end.1].iter().all(|n| n.is_finite()) {
            return false;
        }
        path.curve_to(c1, c2, end);
        last = end;
    }
    true
}

/// Read an SVG arc flag from its accepted boolean or numeric representation.
fn arc_flag(v: &Value, key: &str) -> bool {
    match v.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64() == Some(1),
        _ => false,
    }
}

/// Convert Penpot paths into editable Bézier networks. Commands are absolute page coordinates;
/// node geometry is local, so subtract the shape's x/y exactly once.
fn shape_matrix(v: &Value, x: f64, y: f64, w: f64, h: f64) -> Affine {
    let flip = |key: &str| if v.get(key).and_then(Value::as_bool) == Some(true) { -1.0 } else { 1.0 };
    let fallback = Affine::rotate(-number(v, "rotation", 0.0).to_radians()) * Affine::scale_non_uniform(flip("flipX"), flip("flipY"));
    let linear = v
        .get("transform")
        .map(|t| Affine::new([snap(number(t, "a", 1.0)), snap(number(t, "b", 0.0)), snap(number(t, "c", 0.0)), snap(number(t, "d", 1.0)), 0.0, 0.0]))
        .filter(|m| m.determinant().abs() > 1e-9)
        .unwrap_or(fallback);
    Affine::translate((x + w / 2.0, y + h / 2.0)) * linear * Affine::translate((-w / 2.0, -h / 2.0))
}

/// Find a shape fill inherited from its ancestor metadata.
fn inherited_fill(shapes: &BTreeMap<String, Value>, v: &Value) -> Value {
    let mut parent = string(v, "parentId");
    for _ in 0..MAX_DEPTH {
        let Some(p) = parent.filter(|id| *id != ROOT_ID).and_then(|id| shapes.get(id)) else { break };
        match string(p, "type") {
            Some("frame" | "bool") => break,
            _ if list(p, "fills").next().is_some() => return serde_json::json!({"fills": list(p, "fills").take(1).collect::<Vec<_>>()}),
            Some("group" | "svg-raw") if p.get("svgAttrs").is_some() => return serde_json::json!({"fills": [{"fillColor": "#000000"}]}),
            _ => parent = string(p, "parentId"),
        }
    }
    serde_json::json!({"fills": []})
}

/// Convert a filled path into an editable vector network.
fn filled_network(path: &BezPath) -> VectorNetwork {
    use figcraft_doc::geom::kurbo::PathEl;
    let mut closed = BezPath::new();
    let mut synthetic_ids = BTreeSet::new();
    let mut segments = 0usize;
    let mut started = false;
    let mut open: Option<(figcraft_doc::geom::Point, figcraft_doc::geom::Point)> = None;
    let mut close =
        |closed: &mut BezPath, segments: &mut usize, open: Option<(figcraft_doc::geom::Point, figcraft_doc::geom::Point)>, synthetic: bool| {
            let Some((start, end)) = open else { return };
            closed.close_path();
            if (end - start).hypot() > 1e-6 {
                if synthetic {
                    synthetic_ids.insert(*segments);
                }
                *segments += 1;
            }
        };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                close(&mut closed, &mut segments, open.take(), true);
                closed.move_to(p);
                open = Some((p, p));
                started = true;
            }
            PathEl::ClosePath => close(&mut closed, &mut segments, open.take(), false),
            other => {
                closed.push(other);
                if started {
                    segments += 1;
                }
                if let (Some((start, _)), Some(end)) = (open, other.end_point()) {
                    open = Some((start, end));
                }
            }
        }
    }
    close(&mut closed, &mut segments, open.take(), true);
    let mut net = VectorNetwork::from_path(&closed);
    let kept: Vec<Option<usize>> = (0..net.segments.len())
        .scan(0usize, |next, i| {
            Some(if synthetic_ids.contains(&i) {
                None
            } else {
                *next += 1;
                Some(*next - 1)
            })
        })
        .collect();
    net.segments = net.segments.iter().enumerate().filter(|(i, _)| !synthetic_ids.contains(i)).map(|(_, s)| *s).collect();
    for region in &mut net.regions {
        for lp in &mut region.loops {
            *lp = lp.iter().filter_map(|i| kept.get(*i).copied().flatten()).collect();
        }
    }
    net
}

/// Remove insignificant floating-point noise from transform coefficients.
fn snap(coefficient: f64) -> f64 {
    let whole = coefficient.round();
    if (coefficient - whole).abs() < 1e-5 { whole } else { coefficient }
}

#[cfg(test)]
/// Import vector commands into an editable native network.
fn vector_path(v: &Value, warnings: &mut Vec<String>) -> Option<VectorNetwork> {
    vector_bezpath(v, warnings).map(|path| VectorNetwork::from_path(&path))
}

/// Parse supported vector commands and report unrepresentable geometry.
fn vector_bezpath(v: &Value, warnings: &mut Vec<String>) -> Option<BezPath> {
    if let Some(d) = v.get("content").and_then(Value::as_str) {
        let path = BezPath::from_svg(d).ok().filter(|p| !p.elements().is_empty())?;
        let origin = Affine::translate((-geometry(v, "x", 0.0), -geometry(v, "y", 0.0)));
        return Some(origin * path);
    }
    let commands = v.get("content")?.as_array()?;
    if commands.is_empty() || commands.len() > 100_000 {
        return None;
    }
    let (ox, oy) = (geometry(v, "x", 0.0), geometry(v, "y", 0.0));
    let mut path = BezPath::new();
    let mut current = (0.0, 0.0);
    let mut start = (0.0, 0.0);
    let mut has_move = false;
    let mut last_cubic: Option<(f64, f64)> = None;
    let mut last_quad: Option<(f64, f64)> = None;
    for item in commands {
        let cmd = string(item, "command")?;
        let args = item.get("params").unwrap_or(item);
        let pt = (number(args, "x", current.0 + ox) - ox, number(args, "y", current.1 + oy) - oy);
        match cmd {
            "move-to" | "M" => {
                path.move_to(pt);
                current = pt;
                start = pt;
                has_move = true;
                last_cubic = None;
                last_quad = None;
            }
            "line-to" | "L" if has_move => {
                path.line_to(pt);
                current = pt;
                last_cubic = None;
                last_quad = None;
            }
            "line-to-horizontal" | "H" if has_move => {
                let end = (pt.0, current.1);
                path.line_to(end);
                current = end;
                last_cubic = None;
                last_quad = None;
            }
            "line-to-vertical" | "V" if has_move => {
                let end = (current.0, pt.1);
                path.line_to(end);
                current = end;
                last_cubic = None;
                last_quad = None;
            }
            "curve-to" | "C" if has_move => {
                let c1 = (number(args, "c1x", ox) - ox, number(args, "c1y", oy) - oy);
                let c2 = (number(args, "c2x", ox) - ox, number(args, "c2y", oy) - oy);
                path.curve_to(c1, c2, pt);
                current = pt;
                last_cubic = Some(c2);
                last_quad = None;
            }
            "smooth-curve-to" | "S" if has_move => {
                let c1 = last_cubic.map(|p| (2.0 * current.0 - p.0, 2.0 * current.1 - p.1)).unwrap_or(current);
                let c2 = (number(args, "c2x", ox) - ox, number(args, "c2y", oy) - oy);
                path.curve_to(c1, c2, pt);
                current = pt;
                last_cubic = Some(c2);
                last_quad = None;
            }
            "quadratic-bezier-curve-to" | "Q" if has_move => {
                let c = (number(args, "c1x", ox) - ox, number(args, "c1y", oy) - oy);
                path.quad_to(c, pt);
                current = pt;
                last_quad = Some(c);
                last_cubic = None;
            }
            "smooth-quadratic-bezier-curve-to" | "T" if has_move => {
                let c = last_quad.map(|p| (2.0 * current.0 - p.0, 2.0 * current.1 - p.1)).unwrap_or(current);
                path.quad_to(c, pt);
                current = pt;
                last_quad = Some(c);
                last_cubic = None;
            }
            "close-path" | "Z" if has_move => {
                path.close_path();
                current = start;
                last_cubic = None;
                last_quad = None;
            }
            "elliptical-arc" | "A" if has_move => {
                if !arc_to_cubics(
                    &mut path,
                    current,
                    pt,
                    number(args, "rx", 0.0),
                    number(args, "ry", 0.0),
                    number(args, "xAxisRotation", 0.0),
                    arc_flag(args, "largeArcFlag"),
                    arc_flag(args, "sweepFlag"),
                ) {
                    warn(warnings, "Penpot ellipse arc has invalid parameters; retained as unconverted frame".into());
                    return None;
                }
                current = pt;
                last_cubic = None;
                last_quad = None;
            }
            _ => {
                warn(warnings, format!("Unsupported Penpot path command '{cmd}'; path left unconverted"));
                return None; // Never silently drop segments (that would change the silhouette).
            }
        }
    }
    if !has_move {
        return None;
    }
    Some(path)
}

/// Import bounded shadow and blur effects, warning about unsupported blur types.
fn effects(v: &Value, warnings: &mut Vec<String>) -> Vec<Effect> {
    let mut out = Vec::new();
    for item in list(v, "shadow").chain(list(v, "shadows")).take(32) {
        let raw_color = item.get("color").unwrap_or(item);
        let c = raw_color.as_str().and_then(Color::from_hex).or_else(|| string(raw_color, "color").and_then(Color::from_hex)).unwrap_or(Color::BLACK);
        let opacity = number(raw_color, "opacity", number(item, "opacity", 1.0));
        let shadow = Shadow {
            color: c.with_alpha((c.a as f64 * opacity).clamp(0.0, 1.0) as f32),
            offset: (
                number(item, "offsetX", item.get("offset").map(|o| number(o, "x", 0.0)).unwrap_or(0.0)),
                number(item, "offsetY", item.get("offset").map(|o| number(o, "y", 0.0)).unwrap_or(0.0)),
            ),
            radius: number(item, "blur", 0.0).clamp(0.0, 10000.0),
            spread: number(item, "spread", 0.0).clamp(-10000.0, 10000.0),
            ..Shadow::default()
        };
        let inner = matches!(string(item, "style"), Some("inner-shadow" | "inner"));
        out.push(Effect {
            kind: if inner { EffectKind::InnerShadow(shadow) } else { EffectKind::DropShadow(shadow) },
            visible: item.get("hidden").and_then(Value::as_bool) != Some(true),
        });
    }
    for blur in [v.get("blur"), v.get("backgroundBlur")].into_iter().flatten() {
        let mode = string(blur, "type").unwrap_or("layer-blur");
        let radius = number(blur, "value", 0.0).clamp(0.0, 1000.0);
        let kind = match mode {
            "layer-blur" | "layer" => Some(EffectKind::LayerBlur(Blur::uniform(radius))),
            "background-blur" | "background" => Some(EffectKind::BackgroundBlur(Blur::uniform(radius))),
            _ => None,
        };
        if let Some(kind) = kind {
            out.push(Effect { kind, visible: blur.get("hidden").and_then(Value::as_bool) != Some(true) });
        } else {
            warn(warnings, format!("Unsupported Penpot blur type '{mode}'"));
        }
    }
    out
}

/// Resolve a Penpot image fill through `media/<id>.json` -> `objects/<mediaId>.png|jpg`.
fn paints(
    z: &mut ZipArchive<Cursor<&[u8]>>,
    base: &str,
    shape: &Value,
    key: &str,
    doc: &mut Document,
    images: &mut BTreeMap<String, String>,
    image_bytes: &mut usize,
    budget: &mut InflateBudget,
    warnings: &mut Vec<String>,
) -> Vec<Paint> {
    let mut out = Vec::new();
    for fill in list(shape, key).take(50) {
        let stroke = key == "strokes";
        let opacity = if stroke { number(fill, "strokeOpacity", number(fill, "fillOpacity", 1.0)) } else { number(fill, "fillOpacity", 1.0) }
            .clamp(0.0, 1.0) as f32;
        if let Some(image) = fill.get(if stroke { "strokeImage" } else { "fillImage" }).or_else(|| if stroke { fill.get("fillImage") } else { None })
        {
            let Some(id) = string(image, "id").filter(|id| uuid(id)) else {
                warn(warnings, "Invalid image fill identifier".into());
                continue;
            };
            let resolved = match images.get(id) {
                Some(hash) => Some(hash.clone()),
                None => {
                    let meta_path = format!("{base}/media/{id}.json");
                    let meta = json(z, &meta_path, budget).ok();
                    let object_id = meta.as_ref().and_then(|m| string(m, "mediaId")).filter(|oid| uuid(oid));
                    let extension = match meta.as_ref().and_then(|m| string(m, "mtype")) {
                        Some("image/jpeg") => "jpg",
                        Some("image/webp") => "webp",
                        Some("image/png") => "png",
                        _ => "unknown",
                    };
                    if let Some(obj) = object_id {
                        let p = format!("objects/{obj}.{extension}");
                        let data = z.by_name(&p).ok().and_then(|f| {
                            if f.size() > MAX_IMAGE {
                                return None;
                            }
                            let mut bytes = Vec::new();
                            BudgetReader { inner: f, budget: &mut *budget }.take(MAX_IMAGE + 1).read_to_end(&mut bytes).ok()?;
                            if bytes.len() as u64 > MAX_IMAGE || bytes.len() > MAX_TOTAL_IMAGES.saturating_sub(*image_bytes) {
                                return None;
                            }
                            Some(bytes)
                        });
                        data.and_then(|b| {
                            let size = b.len();
                            let result = add_image(doc, b).ok().map(|(hash, _, _)| hash);
                            if result.is_some() {
                                *image_bytes += size;
                            }
                            result
                        })
                    } else {
                        None
                    }
                }
            };
            if let Some(hash) = resolved {
                images.insert(id.to_owned(), hash.clone());
                // A Penpot image with keepAspectRatio=false stretches the full source
                // rectangle into the layer. FigCraft's Crop mode with identity UV mapping
                // is the native, editable representation (Fill would crop the edges).
                let stretch = image.get("keepAspectRatio").and_then(Value::as_bool) == Some(false);
                let mut p = Paint::image(hash, if stretch { ImageScaleMode::Crop } else { ImageScaleMode::Fill });
                if stretch && let PaintKind::Image(ref mut img) = p.kind {
                    img.crop = Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
                }
                p.opacity = opacity;
                p.visible = fill.get("hidden").and_then(Value::as_bool) != Some(true);
                out.push(p);
            } else {
                warn(warnings, format!("Missing or unsupported Penpot image {id}"));
            }
        } else if let Some(raw) = fill.get(if stroke { "strokeColorGradient" } else { "fillColorGradient" }).or_else(|| fill.get("gradient")) {
            if let Some(g) = gradient(raw) {
                let mut p = Paint::gradient(g);
                p.opacity = opacity;
                p.visible = fill.get("hidden").and_then(Value::as_bool) != Some(true);
                out.push(p);
            } else {
                warn(warnings, "Unrecognized Penpot gradient".into());
            }
        } else if let Some(hex) =
            string(fill, if stroke { "strokeColor" } else { "fillColor" }).or_else(|| if stroke { string(fill, "fillColor") } else { None })
            && let Some(c) = Color::from_hex(hex)
        {
            let mut p = Paint::solid(c);
            p.opacity *= opacity;
            p.visible = fill.get("hidden").and_then(Value::as_bool) != Some(true);
            out.push(p);
        }
    }
    out.reverse();
    out
}

/// Penpot gradient handles are unit-box positions. The `width` factor defines the perpendicular
/// radius of a radial gradient; FigCraft stores that as a perpendicular handle position.
fn gradient(v: &Value) -> Option<Gradient> {
    let kind = match string(v, "type")? {
        "linear" => GradientKind::Linear,
        "radial" => GradientKind::Radial,
        _ => return None,
    };
    let stops = list(v, "stops")
        .take(128)
        .filter_map(|s| {
            let color = string(s, "color").and_then(Color::from_hex)?;
            Some(ColorStop {
                position: number(s, "offset", 0.0).clamp(0.0, 1.0) as f32,
                color: color.with_alpha((color.a as f64 * number(s, "opacity", 1.0).clamp(0.0, 1.0)) as f32),
            })
        })
        .collect::<Vec<_>>();
    if stops.is_empty() {
        return None;
    }
    let from = (number(v, "startX", 0.0), number(v, "startY", 0.0));
    let to = (number(v, "endX", 1.0), number(v, "endY", 0.0));
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let radius = number(v, "width", 1.0).clamp(0.0, 1000.0);
    let mut g = Gradient { kind, stops, from, to, width: (from.0 - dy * radius, from.1 + dx * radius) };
    if kind == GradientKind::Linear {
        g.square_linear_width(1.0, 1.0);
    }
    Some(g)
}

#[cfg(test)]
mod google_font_ref_tests {
    use super::*;
    #[test]
    fn nested_runs_inherit_the_google_font_reference_and_weight() {
        let source = serde_json::json!({"type":"text", "fontId":"gfont-open-sans",
          "content":{"type":"paragraph-set", "fontFamily":"Open Sans", "children":[
            {"type":"paragraph", "fontWeight":700, "children":[
              {"text":"Bold"}, {"text":" Italic", "fontVariantId":"700italic"}
            ]}
          ]}
        });
        let refs = referenced_google_fonts(&source);
        assert_eq!(refs.len(), 1);
        let variants = refs[0]["variants"].as_array().unwrap();
        assert!(variants.contains(&serde_json::json!({"weight":700,"italic":false})));
        assert!(variants.contains(&serde_json::json!({"weight":700,"italic":true})));
    }

    #[test]
    fn variant_id_supplies_weight_when_weight_is_not_explicit() {
        let source = serde_json::json!({"content":{"fontId":"gfont-open-sans", "fontFamily":"Open Sans", "fontVariantId":"700italic", "text":"Hi"}});
        let refs = referenced_google_fonts(&source);
        assert!(refs[0]["variants"].as_array().unwrap().contains(&serde_json::json!({"weight":700,"italic":true})));
    }

    #[test]
    fn text_runs_use_variant_id_for_the_rendered_face() {
        let source = serde_json::json!({"content":{"type":"root", "children":[
            {"text":"Example", "fontId":"gfont-open-sans", "fontFamily":"Open Sans", "fontVariantId":"700italic"}
        ]}});
        let text = text_node(&source);
        assert_eq!(text.style_at(0).font_family, "Open Sans");
        assert_eq!(text.style_at(0).font_style, "Bold Italic");
    }

    #[test]
    fn named_font_weight_is_recorded_for_google_fonts() {
        let source = serde_json::json!({"content": {"fontId":"gfont-roboto", "fontFamily":"Roboto", "fontWeight":"bold", "text":"Hi"}});
        let refs = referenced_google_fonts(&source);
        assert!(refs[0]["variants"].as_array().unwrap().contains(&serde_json::json!({"weight":700,"italic":false})));
    }

    #[test]
    fn preserves_google_font_id_and_family_from_nested_text_runs() {
        let layer = serde_json::json!({"content":{"type":"root","children":[
            {"type":"paragraph-set","children":[{"type":"paragraph","fontId":"gfont-spicy-rice", "fontFamily":"Spicy Rice",
              "children":[{"fontId":"gfont-spicy-rice","fontFamily":"Spicy Rice","text":"demo"}]}]}
        ]}});
        let refs = referenced_google_fonts(&layer);
        assert_eq!(refs, vec![serde_json::json!({"id":"gfont-spicy-rice","family":"Spicy Rice", "variants":[{"weight":400,"italic":false}]})]);
    }
    #[test]
    fn preserves_multiple_weights_and_italic_variants_in_text_runs() {
        let layer = serde_json::json!({"content":{"children":[
            {"fontId":"gfont-open-sans","fontFamily":"Open Sans", "fontWeight":"700", "fontVariantId":"700italic"},
            {"fontId":"gfont-open-sans","fontFamily":"Open Sans", "fontWeight":400, "fontVariantId":"regular"}
        ]}});
        let refs = referenced_google_fonts(&layer);
        assert_eq!(
            refs,
            vec![serde_json::json!({"id":"gfont-open-sans","family":"Open Sans", "variants":[
                {"weight":400,"italic":false}, {"weight":700,"italic":true}
            ]})]
        );
    }

    #[test]
    fn preserves_custom_font_id_separately_from_google_provider() {
        let source = serde_json::json!({"content":{"children":[
            {"text":"Playball", "fontId":"custom-407a1221-a42c-80cc-8008-cd8ae87156cc", "fontFamily":"Playball", "fontVariantId":"normal-400"},
            {"text":"Playball", "fontId":"gfont-playball", "fontFamily":"Playball", "fontVariantId":"regular"}
        ]}});
        let custom = referenced_custom_fonts(&source);
        assert_eq!(custom.len(), 1);
        assert_eq!(custom[0]["id"], "custom-407a1221-a42c-80cc-8008-cd8ae87156cc");
        assert_eq!(custom[0]["variants"][0]["weight"], 400);
        assert_eq!(referenced_google_fonts(&source).len(), 1);
    }

    #[test]
    fn ignores_untrusted_or_unresolvable_font_ids() {
        let v = serde_json::json!({"content":{"children":[
            {"fontId":"custom-team-font", "fontFamily":"Spicy Rice"},
            {"fontId":"gfont-../bad", "fontFamily":"Spicy Rice"},
            {"fontId":"gfont-spicy-rice", "fontFamily":"Spicy Rice"}
        ]}});
        assert_eq!(referenced_google_fonts(&v).len(), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflate_budget_counts_bytes_read_even_when_headers_claim_less() {
        // Two independent readers share the same actual output limit.
        let mut budget = InflateBudget { remaining: 5, ..InflateBudget::new() };
        let mut first = Vec::new();
        BudgetReader { inner: Cursor::new(b"abcd"), budget: &mut budget }.read_to_end(&mut first).unwrap();
        assert_eq!(first, b"abcd");
        assert_eq!(budget.remaining, 1);

        let mut second = Vec::new();
        let result = BudgetReader { inner: Cursor::new(b"xyz"), budget: &mut budget }.read_to_end(&mut second);
        assert!(result.is_err(), "bytes beyond the combined budget must be rejected");
        assert_eq!(budget.remaining, 0);
        assert!(budget.check().is_err());
    }

    #[test]
    fn json_value_budget_is_shared_by_shape_and_other_members() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for name in ["tokens.json", "shape.json"] {
            writer.start_file(name, SimpleFileOptions::default()).unwrap();
            writer.write_all(b"[0,0]").unwrap();
        }
        let bytes = writer.finish().unwrap().into_inner();
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let mut budget = InflateBudget { json_values: 6, ..InflateBudget::new() };
        let mut page_bytes = MAX_PAGE_SHAPES_BYTES;
        assert_eq!(json(&mut archive, "tokens.json", &mut budget).unwrap(), serde_json::json!([0, 0]));
        assert_eq!(shape_json(&mut archive, "shape.json", &mut budget, &mut page_bytes).unwrap(), serde_json::json!([0, 0]));
        assert_eq!(budget.json_values, 0);
        assert!(budget.check().is_ok(), "an exact-budget import is allowed");
        assert!(json(&mut archive, "tokens.json", &mut budget).is_err());
        assert!(budget.check().is_err(), "optional-member callers cannot swallow exhaustion");
    }

    #[test]
    fn json_value_budget_rejects_dense_input_before_deserialization() {
        // The malformed tail would cause a serde syntax error after allocating the array.
        // Admission must reject it for structure first, while the input is still only bytes.
        let dense = "[".to_owned() + &"0,".repeat(10_000);
        let mut budget = InflateBudget { json_values: 100, ..InflateBudget::new() };
        let err = budget.parse_json(dense.as_bytes(), "shape.json").unwrap_err();
        assert!(err.to_string().contains("value budget"), "{err}");
        assert!(budget.check().is_err());
    }

    #[test]
    fn json_value_budget_handles_strings_escapes_and_empty_input() {
        for text in ["[{,:", "\\", "\\\"[{,:", "é"] {
            let value = Value::String(text.into());
            let mut budget = InflateBudget { json_values: 1, ..InflateBudget::new() };
            assert_eq!(budget.parse_json(&serde_json::to_vec(&value).unwrap(), "shape.json").unwrap(), value);
            assert!(budget.check().is_ok());
        }
        // An escaped backslash before a closing quote must not hide later containers.
        let mut budget = InflateBudget { json_values: 3, ..InflateBudget::new() };
        assert!(budget.parse_json(br#"["\\",[0,0]]"#, "shape.json").is_err());
        assert!(budget.check().is_err());
        let mut budget = InflateBudget { json_values: 0, ..InflateBudget::new() };
        assert!(budget.parse_json(b"", "shape.json").is_err());
        assert!(budget.check().is_err());
    }

    #[test]
    fn import_rejects_a_dense_shape_within_the_zip_byte_limits() {
        let id = "11111111-1111-1111-1111-111111111111";
        let bytes = archive_with_shapes(&[(id, serde_json::json!({"id":id,"type":"rect"}))]);
        let mut source = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let dense = format!(r#"{{"id":"{id}","type":"rect","unused":[{}0]}}"#, "0,".repeat(MAX_JSON_VALUES));
        assert!(dense.len() < MAX_JSON as usize);
        for index in 0..source.len() {
            let mut member = source.by_index(index).unwrap();
            writer.start_file(member.name(), crate::json_options()).unwrap();
            if member.name().ends_with(&format!("/{id}.json")) {
                writer.write_all(dense.as_bytes()).unwrap();
            } else {
                io::copy(&mut member, &mut writer).unwrap();
            }
        }
        let bytes = writer.finish().unwrap().into_inner();
        assert!(bytes.len() < MAX_ARCHIVE);
        let err = import(&bytes).unwrap_err();
        assert!(err.to_string().contains("value budget"), "{err}");
    }

    #[test]
    fn indexed_token_preserves_set_priority_and_charges_probes() {
        let original = serde_json::json!({"$type":"number", "$value":1});
        let latest = serde_json::json!({"$type":"number", "$value":3});
        let paths = BTreeMap::from([("size", vec![(0, &original), (2, &latest)])]);
        let mut budget = TokenResolutionBudget { remaining: 2 };
        assert_eq!(indexed_token("size", &BTreeSet::from([0]), &paths, &mut budget), Some(&original));
        assert_eq!(budget.remaining, 0, "both candidates must consume work");
        assert_eq!(indexed_token("size", &BTreeSet::from([2]), &paths, &mut budget), None);
    }

    #[test]
    fn token_number_charges_each_missing_set_in_linear_lookup() {
        let number = serde_json::json!({"$type":"number", "$value":7});
        let tokens = BTreeMap::from([("A", BTreeMap::from([("size".to_owned(), &number)])), ("B", BTreeMap::new()), ("C", BTreeMap::new())]);
        let mut small = TokenResolutionBudget { remaining: 3 };
        assert_eq!(token_number("size", &["A", "B", "C"], &tokens, 0, 0, &mut small), None);
        let mut enough = TokenResolutionBudget { remaining: 4 };
        assert_eq!(token_number("size", &["A", "B", "C"], &tokens, 0, 0, &mut enough), Some(7.0));
    }

    #[test]
    fn grid_cells_indexes_child_shapes_to_their_cell() {
        let parent = serde_json::json!({
            "layoutGridCells": {
                "cell-1": {"row": 1, "col": 1, "shapes": ["child-a", "child-b"]},
                "cell-2": {"row": 1, "col": 2, "shapes": ["child-c"]}
            }
        });
        let cells = grid_cells(&parent);
        assert_eq!(cells.get("child-a").and_then(|c| c.get("row")).and_then(Value::as_u64), Some(1));
        assert_eq!(cells.get("child-b").and_then(|c| c.get("col")).and_then(Value::as_u64), Some(1));
        assert_eq!(cells.get("child-c").and_then(|c| c.get("col")).and_then(Value::as_u64), Some(2));
        assert_eq!(cells.get("child-unknown"), None);

        let non_grid = serde_json::json!({"type": "frame"});
        assert!(grid_cells(&non_grid).is_empty());
    }

    #[test]
    fn page_shape_budget_rejects_shapes_exceeding_byte_limit() {
        let buffer = Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(buffer);
        z.start_file("shape.json", SimpleFileOptions::default()).unwrap();
        z.write_all(br#"{"id":"00000000-0000-0000-0000-000000000001","type":"rect"}"#).unwrap();
        let bytes = z.finish().unwrap().into_inner();
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();

        let mut inflate_budget = InflateBudget::new();
        let mut page_budget = 10u64; // Too small for the shape JSON (~60 bytes)
        let res = shape_json(&mut archive, "shape.json", &mut inflate_budget, &mut page_budget);
        assert!(res.is_err());

        let mut archive2 = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let mut inflate_budget2 = InflateBudget::new();
        let mut page_budget2 = 1000u64;
        let res2 = shape_json(&mut archive2, "shape.json", &mut inflate_budget2, &mut page_budget2);
        assert!(res2.is_ok());
        assert!(page_budget2 < 1000);
    }
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn archive_with_shapes(shapes: &[(&str, Value)]) -> Vec<u8> {
        archive_with_components(shapes, &[])
    }

    fn archive_with_components(shapes: &[(&str, Value)], components: &[(&str, Value)]) -> Vec<u8> {
        archive_with_page(shapes, components, Value::Null)
    }

    /// The same archive, with the page's own JSON (its `flows` among them).
    fn archive_with_page(shapes: &[(&str, Value)], components: &[(&str, Value)], page_json: Value) -> Vec<u8> {
        let file = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
        let page = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
        let ids = shapes
            .iter()
            .filter(|(_, shape)| string(shape, "parentId").is_none_or(|parent| parent == ROOT_ID))
            .map(|(id, _)| Value::String((*id).into()))
            .collect::<Vec<_>>();
        let buffer = Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(buffer);
        let mut entries = vec![
            ("manifest.json".into(), serde_json::json!({ "type": "penpot/export-files", "version": 1, "files": [{"id": file, "name": "Advanced"}]})),
            (
                format!("files/{file}/pages/{page}.json"),
                if page_json.is_null() { serde_json::json!({"id":page,"name":"Canvas","index":0}) } else { page_json },
            ),
            (format!("files/{file}/pages/{page}/{ROOT_ID}.json"), serde_json::json!({"id":ROOT_ID,"type":"frame","shapes":ids})),
        ];
        entries.extend(shapes.iter().map(|(id, value)| (format!("files/{file}/pages/{page}/{id}.json"), value.clone())));
        entries.extend(components.iter().map(|(id, value)| (format!("files/{file}/components/{id}.json"), value.clone())));
        for (name, data) in entries {
            z.start_file(name, SimpleFileOptions::default()).unwrap();
            z.write_all(data.to_string().as_bytes()).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    #[test]
    fn raw_svg_from_export_is_preserved_for_native_svg_expansion() {
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let raw = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="25"><path d="M0 0L40 25" stroke="red"/></svg>"#;
        let layer = serde_json::json!({"id":id,"type":"svg-raw","name":"Vector ornament","width":40,"height":25,
            "svgAttrs":{"raw":raw}});
        let (doc, issues) = import(&archive_with_shapes(&[(id, layer)])).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        let node = doc.nodes().find(|n| n.name == "Vector ornament").unwrap();
        assert_eq!(node.plugin_data.get("penpot.svgRawXml").map(String::as_str), Some(raw));
        assert_eq!(node.plugin_data.get("penpot.svgRawStatus").map(String::as_str), Some("pending"));
    }

    #[test]
    fn raw_svg_json_tree_escapes_attribute_values_and_text() {
        let shape = serde_json::json!({"type":"svg-raw","width":80,"height":40,
            "content":{"tag":"g","attrs":{"id":"A&B","onclick":"bad()"},
                "content":[{"tag":"text","content":"Hello <design> & more"}]}});
        let xml = raw_svg_xml(&shape).unwrap();
        assert!(xml.starts_with("<svg xmlns="));
        assert!(xml.contains("id=\"A&amp;B\""));
        assert!(xml.contains("Hello &lt;design&gt; &amp; more"));
        assert!(!xml.contains("onclick"));
    }

    #[test]
    fn raw_svg_rejects_oversize_markup_and_invalid_tree_tags() {
        let oversized = serde_json::json!({"svgAttrs":{"raw":format!("<svg>{}</svg>", "x".repeat(131072))}});
        assert!(raw_svg_xml(&oversized).is_none());
        let malformed = serde_json::json!({"content":{"tag":"x><script","content":"bad"}});
        assert!(raw_svg_xml(&malformed).is_none());
    }

    #[test]
    fn text_styles_inherit_from_nested_paragraphs_without_rasterizing() {
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let text = serde_json::json!({"id":id,"name":"Paragraph inheritance","type":"text",
            "content":{"type":"root","fontFamily":"Inter","children":[
                {"type":"paragraph","fontSize":28,"textAlign":"center","children":[
                    {"type":"span","fontWeight":"700","children":[{"text":"Strong "}]},
                    {"text":"normal"}
                ]}
            ]}
        });
        let (doc, warnings) = import(&archive_with_shapes(&[(id, text)])).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let n = doc.nodes().find(|n| n.name == "Paragraph inheritance").unwrap();
        let NodeKind::Text(text) = &n.kind else {
            panic!("expected text");
        };
        assert_eq!(text.characters, "Strong normal");
        assert_eq!(text.style.font_family, "Inter");
        assert_eq!(text.align_h, TextAlignH::Center);
        assert_eq!(text.runs.len(), 2);
        assert_eq!((text.runs[0].start, text.runs[0].end), (0, 7));
        assert_eq!(text.runs[0].style.font_style, "Bold");
        assert_eq!(text.runs[0].style.font_size, 28.0);
        assert_eq!((text.runs[1].start, text.runs[1].end), (7, 13));
        assert_eq!(text.runs[1].style.font_style, "Regular");
        assert_eq!(text.runs[1].style.font_size, 28.0);
    }

    #[test]
    fn page_guides_support_object_and_array_forms_without_invalid_coordinates() {
        let guides = serde_json::json!({
            "one": {"axis":"x","position":120.5},
            "two": {"axis":"y","position":-40},
            "invalid": {"axis":"diagonal","position":10},
            "other": {"axis":"x","position":"NaN"}
        });
        let parsed = penpot_guides(Some(&guides));
        assert_eq!(parsed.len(), 2);
        assert!(parsed.contains(&Guide { vertical: true, offset: 120.5 }));
        assert!(parsed.contains(&Guide { vertical: false, offset: -40.0 }));
        let array = serde_json::json!([{"orientation":"vertical","offset":"25"}]);
        assert_eq!(penpot_guides(Some(&array)), vec![Guide { vertical: true, offset: 25.0 }]);
    }

    #[test]
    fn board_display_grids_become_editable_native_grids() {
        let shape = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let frame = serde_json::json!({"id":shape,"type":"frame","name":"Board", "grids":[
            {"type":"square","display":true,"params":{"size":8,"color":{"color":"#00FF00","opacity":0.25}}},
            {"type":"column","display":false,"params":{"count":12,"gutter":24,"margin":16,"type":"center"}},
            {"type":"rows","params":{"count":4,"size":30,"gutter":8}}
        ]});
        let (doc, warnings) = import(&archive_with_shapes(&[(shape, frame)])).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let board = doc.nodes().find(|n| n.name == "Board").unwrap();
        assert_eq!(board.grids.len(), 3);
        assert_eq!(board.grids[0].kind, GridKind::Grid);
        assert_eq!(board.grids[0].size, 8.0);
        assert!((board.grids[0].color.a - 0.25).abs() < 0.01);
        assert_eq!(board.grids[1].kind, GridKind::Columns);
        assert_eq!(board.grids[1].count, 12);
        assert!(!board.grids[1].visible);
        assert_eq!(board.grids[1].align, GridAlign::Center);
        assert_eq!(board.grids[2].kind, GridKind::Rows);
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn masked_group_reorders_only_an_explicit_unique_mask() {
        let group = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let mask = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let content = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let shapes = [
            (group, serde_json::json!({"id":group,"name":"Masked","type":"group", "maskedGroup":true,"shapes":[mask, content]})),
            (mask, serde_json::json!({"id":mask,"name":"Mask","type":"rect","mask":true,"parentId":group})),
            (content, serde_json::json!({"id":content,"name":"Content","type":"circle","parentId":group})),
        ];
        let (doc, warnings) = import(&archive_with_shapes(&shapes)).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let group = doc.nodes().find(|n| n.name == "Masked").unwrap();
        let names = group.children.iter().map(|&id| doc.node(id).unwrap().name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["Mask", "Content"]);
        assert!(doc.node(group.children[0]).unwrap().is_mask);
        assert_eq!(group.plugin_data.get("penpot.maskImported").map(String::as_str), Some("true"));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn masked_group_uses_its_first_layer_as_the_mask_like_penpot() {
        let group = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let mask = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let image = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let flagged = "ffffffff-ffff-ffff-ffff-ffffffffffff";
        let shapes = [
            (group, serde_json::json!({"id":group,"name":"Card","type":"group","maskedGroup":true,"shapes":[mask, image, flagged]})),
            (mask, serde_json::json!({"id":mask,"name":"cover-mask","type":"rect","parentId":group})),
            (image, serde_json::json!({"id":image,"name":"Image","type":"rect","parentId":group})),
            (flagged, serde_json::json!({"id":flagged,"name":"Flagged","type":"rect","mask":true,"parentId":group})),
        ];
        let (doc, warnings) = import(&archive_with_shapes(&shapes)).unwrap();
        assert!(!warnings.iter().any(|n| n.contains("mask")), "{warnings:?}");
        let group = doc.nodes().find(|n| n.name == "Card").unwrap();
        let layers = group.children.iter().map(|&id| doc.node(id).unwrap()).map(|n| (n.name.as_str(), n.is_mask)).collect::<Vec<_>>();
        assert_eq!(layers, [("cover-mask", true), ("Image", false), ("Flagged", false)]);
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn masked_group_without_layers_is_preserved_with_a_note() {
        let group = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let shapes = [(group, serde_json::json!({"id":group,"name":"Empty","type":"group","maskedGroup":true,"shapes":[]}))];
        let (doc, warnings) = import(&archive_with_shapes(&shapes)).unwrap();
        assert!(doc.nodes().any(|n| n.name == "Empty"));
        assert!(warnings.iter().any(|n| n.contains("no first layer")));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn exported_pages_and_color_library_survive_native_import() {
        let file = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
        let page = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
        let color = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let buffer = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(buffer);
        let entries = [
            (
                "manifest.json".to_owned(),
                serde_json::json!({"type":"penpot/export-files","version":1,"files":[{"id":file,"name":"Colors and guides"}]}),
            ),
            (
                format!("files/{file}/pages/{page}.json"),
                serde_json::json!({"id":page,"name":"Canvas","index":0,"options":{"guides":{"guide-1":{"axis":"x","position":100}}}}),
            ),
            (format!("files/{file}/pages/{page}/{ROOT_ID}.json"), serde_json::json!({"id":ROOT_ID,"type":"frame","shapes":[]})),
            (format!("files/{file}/colors/{color}.json"), serde_json::json!({"id":color,"name":"Accent","color":"#DEADBE","opacity":0.5})),
        ];
        for (name, value) in entries {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(value.to_string().as_bytes()).unwrap();
        }
        let (doc, warnings) = import(&zip.finish().unwrap().into_inner()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(doc.document_colors.len(), 1);
        assert!((doc.document_colors[0].a - 0.5).abs() < 0.01);
        let page = doc.node(doc.pages[0]).unwrap();
        assert_eq!(page.guides, vec![Guide { vertical: true, offset: 100.0 }]);
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn stretched_image_from_a_penpot_archive_is_editable_without_cropping() {
        const PNG: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73,
            68, 65, 84, 120, 156, 99, 8, 168, 88, 0, 0, 2, 132, 1, 105, 50, 45, 167, 126, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ];
        let file = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
        let page = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
        let shape = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let media = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let object = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let entries: Vec<(String, Vec<u8>)> = vec![
            (
                "manifest.json".into(),
                serde_json::to_vec(&serde_json::json!({
                    "type":"penpot/export-files", "version":1, "files":[{"id":file,"name":"image test"}]
                }))
                .unwrap(),
            ),
            (
                format!("files/{file}/pages/{page}.json"),
                serde_json::to_vec(&serde_json::json!({
                    "id":page,"name":"Canvas","index":0
                }))
                .unwrap(),
            ),
            (
                format!("files/{file}/pages/{page}/{ROOT_ID}.json"),
                serde_json::to_vec(&serde_json::json!({
                    "id":ROOT_ID,"type":"frame","shapes":[shape]
                }))
                .unwrap(),
            ),
            (
                format!("files/{file}/pages/{page}/{shape}.json"),
                serde_json::to_vec(&serde_json::json!({
                    "id":shape,"type":"image","name":"Stretched","width":100,"height":100,
                    "metadata":{"id":media,"keepAspectRatio":false}
                }))
                .unwrap(),
            ),
            (
                format!("files/{file}/media/{media}.json"),
                serde_json::to_vec(&serde_json::json!({
                    "id":media,"mtype":"image/png","mediaId":object
                }))
                .unwrap(),
            ),
            (format!("objects/{object}.png"), PNG.to_vec()),
        ];
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in entries {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(&data).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        let (doc, warnings) = import(&bytes).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(doc.images.len(), 1);
        let node = doc.nodes().find(|n| n.name == "Stretched").unwrap();
        assert!(matches!(node.fills.first().map(|p| &p.kind), Some(PaintKind::Image(img))
            if img.scale_mode == ImageScaleMode::Crop && img.crop == Some([1.0,0.0,0.0,1.0,0.0,0.0])));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn crop_identity_is_the_native_equivalent_of_stretched_penpot_image() {
        // FigCraft Crop maps unit coordinates into image UV. The identity transform
        // stretches the full image to the node regardless of aspect ratio.
        let mut p = Paint::image("source", ImageScaleMode::Crop);
        if let PaintKind::Image(ref mut img) = p.kind {
            img.crop = Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
            assert_eq!(img.effective_crop(640.0, 320.0, 100.0, 100.0), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        } else {
            panic!("expected an image paint");
        }
    }

    #[test]
    fn penpot_svg_arcs_become_native_editable_bezier_segments() {
        let mut warnings = Vec::new();
        let path = serde_json::json!({
            "x": 10, "y": 20,
            "content": [
                {"command": "M", "params": {"x": 10, "y": 20}},
                {"command": "A", "params": {
                    "x": 110, "y": 120, "rx": 100, "ry": 100,
                    "xAxisRotation": 0, "largeArcFlag": false, "sweepFlag": true
                }}
            ]
        });
        let net = vector_path(&path, &mut warnings).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(!net.segments.is_empty());
        assert!(net.segments.iter().any(|seg| !seg.is_straight()));
        let end = net.vertices.last().unwrap();
        assert!((end.x - 100.0).abs() < 1e-7);
        assert!((end.y - 100.0).abs() < 1e-7);
    }

    #[test]
    fn penpot_arc_handles_rotated_ellipse_and_invalid_zero_radius() {
        let mut warnings = Vec::new();
        let path = serde_json::json!({"content": [
            {"command": "move-to", "params": {"x": 10, "y": 10}},
            {"command": "elliptical-arc", "params": {
                "x": 120, "y": 80, "rx": 40, "ry": 15,
                "xAxisRotation": 38, "largeArcFlag": 1, "sweepFlag": 0
            }},
            {"command": "A", "params": {"x": 150, "y": 80, "rx": 0, "ry": 12}}
        ]});
        let net = vector_path(&path, &mut warnings).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(net.segments.iter().any(|s| !s.is_straight()));
        assert!(net.segments.iter().any(|s| s.is_straight()));
        assert!((net.vertices.last().unwrap().x - 150.0).abs() < 1e-7);
    }

    #[test]
    fn omitted_nested_shape_list_recovers_child_from_parent_id() {
        let frame = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let child = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let root = serde_json::json!({
            "id": frame, "name": "Container", "type": "frame", "width": 200, "height": 120,
            "parentId": ROOT_ID, "shapes": []
        });
        let leaf = serde_json::json!({
            "id": child, "name": "Unlisted child", "type": "rect", "parentId": frame,
            "x": 12, "y": 24, "width": 20, "height": 30
        });
        let (doc, warnings) = import(&archive_with_shapes(&[(frame, root), (child, leaf)])).unwrap();
        let parent = doc.nodes().find(|n| n.name == "Container").unwrap();
        let child = doc.nodes().find(|n| n.name == "Unlisted child").unwrap();
        assert_eq!(child.parent, Some(parent.id));
        assert!(parent.children.contains(&child.id));
        assert!(warnings.iter().any(|w| w.contains("omitted a child")));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn root_stacking_list_does_not_lose_unlisted_top_level_layers() {
        let one = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let two = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let rect_a = serde_json::json!({"id":one,"name":"A","type":"rect","parentId":ROOT_ID});
        let rect_b = serde_json::json!({"id":two,"name":"B","type":"rect","parentId":ROOT_ID});
        let mut bytes = archive_with_shapes(&[(one, rect_a), (two, rect_b)]);
        // Build a slightly damaged but valid ZIP whose root names only one of two siblings.
        // Start from a fixture to keep all other structural fields in the original format.
        let mut z = ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut members = Vec::new();
        for i in 0..z.len() {
            let mut e = z.by_index(i).unwrap();
            let mut content = Vec::new();
            e.read_to_end(&mut content).unwrap();
            if e.name().ends_with(&format!("/{ROOT_ID}.json")) {
                let mut v: Value = serde_json::from_slice(&content).unwrap();
                v["shapes"] = serde_json::json!([one]);
                content = serde_json::to_vec(&v).unwrap();
            }
            members.push((e.name().to_string(), content));
        }
        drop(z);
        let cursor = Cursor::new(Vec::new());
        let mut out = zip::ZipWriter::new(cursor);
        for (name, data) in members {
            out.start_file(name, SimpleFileOptions::default()).unwrap();
            out.write_all(&data).unwrap();
        }
        bytes = out.finish().unwrap().into_inner();
        let (doc, warnings) = import(&bytes).unwrap();
        assert!(doc.nodes().any(|n| n.name == "A"));
        assert!(doc.nodes().any(|n| n.name == "B"));
        assert!(warnings.iter().any(|w| w.contains("top-level child")));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn flex_layout_and_item_sizing_import_as_native_layout() {
        let frame = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let child = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let f = serde_json::json!({
            "id":frame, "type":"frame", "name":"Flex board", "x":100, "y":200,
            "width":500, "height":200, "shapes":[child], "layout":"flex",
            "layoutFlexDir":"row", "layoutGap":{"columnGap":12,"rowGap":7},
            "layoutPadding":{"p1":2,"p2":3,"p3":4,"p4":5},
            "layoutJustifyContent":"space-between", "layoutAlignItems":"center",
            "layoutWrapType":"wrap"
        });
        let c = serde_json::json!({
            "id":child, "type":"rect", "name":"Flexible", "parentId":frame,
            "x":120,"y":230,"width":200,"height":80,
            "layoutItemHSizing":"fill", "layoutItemVSizing":"auto",
            "layoutItemMinW":100,"layoutItemAbsolute":false
        });
        let (doc, warnings) = import(&archive_with_shapes(&[(frame, f), (child, c)])).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let f = doc.nodes().find(|n| n.name == "Flex board").unwrap();
        let layout = f.layout.as_ref().unwrap();
        assert_eq!(layout.mode, LayoutMode::Horizontal);
        assert_eq!(layout.gap, 12.0);
        assert_eq!(layout.cross_gap, 7.0);
        assert_eq!(layout.padding, [2.0, 3.0, 4.0, 5.0]);
        assert_eq!(layout.main_align, MainAlign::Min);
        assert_eq!(layout.cross_align, CrossAlign::Center);
        assert!(layout.wrap);
        let c = doc.nodes().find(|n| n.name == "Flexible").unwrap();
        assert_eq!(c.child_layout.sizing_h, Sizing::Fill);
        assert_eq!(c.child_layout.sizing_v, Sizing::Hug);
        assert_eq!(c.child_layout.min_w, Some(100.0));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn space_between_is_kept_only_while_no_child_grows() {
        let frame = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let child = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let row = |sizing: &str| {
            let f = serde_json::json!({"id":frame,"type":"frame","name":"Row","width":300,"height":50,"shapes":[child],
                "layout":"flex","layoutFlexDir":"row","layoutJustifyContent":"space-between"});
            let c = serde_json::json!({"id":child,"parentId":frame,"type":"rect","name":"Item","width":50,"height":50,"layoutItemHSizing":sizing});
            let (doc, _) = import(&archive_with_shapes(&[(frame, f), (child, c)])).unwrap();
            doc.nodes().find(|n| n.name == "Row").unwrap().layout.as_ref().unwrap().main_align
        };
        assert_eq!(row("fix"), MainAlign::SpaceBetween);
        assert_eq!(row("fill"), MainAlign::Min);
    }

    #[test]
    fn stretched_auto_grid_tracks_share_free_space_like_css() {
        let frame = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let wide = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let narrow = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let f = serde_json::json!({
            "id":frame,"type":"frame","name":"Grid","width":430,"height":100,"shapes":[wide,narrow],
            "layout":"grid","layoutJustifyContent":"stretch","layoutAlignContent":"start","layoutJustifyItems":"center",
            "layoutGap":{"columnGap":10,"rowGap":0},"layoutPadding":{"p1":0,"p2":10,"p3":0,"p4":10},
            "layoutGridColumns":[{"type":"auto"},{"type":"auto"},{"type":"fixed","value":100}],
            "layoutGridRows":[{"type":"auto"}],
            "layoutGridCells":{
                "11111111-1111-1111-1111-111111111111":{"shapes":[wide],"row":1,"column":1},
                "22222222-2222-2222-2222-222222222222":{"shapes":[narrow],"row":1,"column":2}
            }
        });
        let w = serde_json::json!({"id":wide,"parentId":frame,"type":"rect","name":"Wide","width":150,"height":40});
        let n = serde_json::json!({"id":narrow,"parentId":frame,"type":"rect","name":"Narrow","width":50,"height":40});
        let (doc, warnings) = import(&archive_with_shapes(&[(frame, f), (wide, w), (narrow, n)])).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let layout = doc.nodes().find(|n| n.name == "Grid").unwrap().layout.clone().unwrap();
        assert_eq!(layout.column_tracks, [GridTrack::fill(195.0), GridTrack::fill(95.0), GridTrack::fixed(100.0)]);
        assert_eq!(layout.row_tracks, [GridTrack::hug()]);
        assert_eq!(layout.main_align, MainAlign::Center);
    }

    #[test]
    fn grid_tracks_and_one_based_cell_positions_are_mapped() {
        let frame = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let child = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let cell = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let f = serde_json::json!({
            "id":frame,"type":"frame","name":"Grid board", "width":320,"height":180,
            "shapes":[child], "layout":"grid", "layoutGap":{"rowGap":9,"columnGap":8},
            "layoutGridColumns":[{"type":"fixed","value":100},{"type":"flex","value":1}],
            "layoutGridRows":[{"type":"auto"}],
            "layoutGridCells":{cell:{"shapes":[child],"row":1,"column":2,"rowSpan":1,"columnSpan":1,"alignSelf":"center"}}
        });
        let c = serde_json::json!({"id":child,"parentId":frame,"type":"rect","name":"Cell","width":20,"height":20});
        let (doc, warnings) = import(&archive_with_shapes(&[(frame, f), (child, c)])).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let n = doc.nodes().find(|n| n.name == "Grid board").unwrap();
        let layout = n.layout.as_ref().unwrap();
        assert_eq!(layout.mode, LayoutMode::Grid);
        assert_eq!((layout.grid_columns, layout.grid_rows), (2, 1));
        assert_eq!(layout.column_tracks[0], GridTrack::fixed(100.0));
        assert_eq!(layout.column_tracks[1], GridTrack::fill(1.0));
        assert_eq!(layout.row_tracks[0], GridTrack::hug());
        let c = doc.nodes().find(|n| n.name == "Cell").unwrap();
        assert_eq!((c.child_layout.grid_row, c.child_layout.grid_column), (Some(0), Some(1)));
        assert_eq!(c.child_layout.grid_align_v, Some(CellAlign::Center));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn masters_and_simple_copies_become_native_linked_instances() {
        let master = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let copy = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let component = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let shapes = [
            (
                master,
                serde_json::json!({"id":master,"name":"Primary button","type":"frame","width":100,"height":40,"componentId":component,"mainInstance":true}),
            ),
            (
                copy,
                serde_json::json!({"id":copy,"name":"Button copy","type":"frame","x":120,"width":100,"height":40,"componentId":component,"shapeRef":master}),
            ),
        ];
        let components = [(component, serde_json::json!({"id":component,"name":"Button","mainInstanceId":master}))];
        let (doc, warnings) = import(&archive_with_components(&shapes, &components)).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let original = doc.nodes().find(|n| n.name == "Primary button").unwrap();
        assert!(matches!(original.kind, NodeKind::Component { .. }));
        assert_eq!(original.plugin_data.get("penpot.componentMasterId").map(String::as_str), Some(component));
        let copy = doc.nodes().find(|n| n.name == "Button copy").unwrap();
        assert!(matches!(copy.kind, NodeKind::Instance { component, .. } if component == original.id));
        assert_eq!(copy.source, Some(original.id));
        assert_eq!(copy.plugin_data.get("penpot.componentLinkStatus").map(String::as_str), Some("linked"));
        assert_eq!(copy.plugin_data.get("penpot.shapeRef").map(String::as_str), Some(master));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn variant_container_becomes_a_component_set_with_property_names() {
        let set = "10000000-0000-0000-0000-000000000001";
        let small = "10000000-0000-0000-0000-000000000002";
        let large = "10000000-0000-0000-0000-000000000003";
        let (small_c, large_c) = ("20000000-0000-0000-0000-000000000002", "20000000-0000-0000-0000-000000000003");
        let main = |id: &str, component: &str, x: i32| {
            serde_json::json!({"id":id,"name":"Star","type":"frame","parentId":set,"x":x,"width":40,"height":40,
            "componentId":component,"mainInstance":true,"componentRoot":true,"variantId":set})
        };
        let shapes = [
            (
                set,
                serde_json::json!({"id":set,"name":"Basic / Star","type":"frame","isVariantContainer":true,"width":200,"height":60,"shapes":[small,large]}),
            ),
            (small, main(small, small_c, 0)),
            (large, main(large, large_c, 60)),
        ];
        let props = |size: &str| serde_json::json!([{"name":"size","value":size},{"name":"color","value":"white, bright"}]);
        let components = [
            (small_c, serde_json::json!({"id":small_c,"name":"Star","mainInstanceId":small,"variantId":set,"variantProperties":props("small")})),
            (large_c, serde_json::json!({"id":large_c,"name":"Star","mainInstanceId":large,"variantId":set,"variantProperties":props("large")})),
        ];
        let (doc, warnings) = import(&archive_with_components(&shapes, &components)).unwrap();
        assert!(!warnings.iter().any(|w| w.contains("variant layers")), "{warnings:?}");
        let set = doc.nodes().find(|n| n.name == "Basic / Star").unwrap();
        assert!(matches!(set.kind, NodeKind::ComponentSet { .. }));
        let names: BTreeSet<&str> = set.children.iter().filter_map(|c| doc.node(*c)).map(|n| n.name.as_str()).collect();
        assert_eq!(names, BTreeSet::from(["size=small, color=white  bright", "size=large, color=white  bright"]));
        assert!(
            set.children.iter().filter_map(|c| doc.node(*c)).all(|n| n.plugin_data.get("penpot.originalName").map(String::as_str) == Some("Star"))
        );
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn copies_of_mains_with_nested_instances_link_with_native_nested_instances() {
        let (icon, icon_c) = ("30000000-0000-0000-0000-000000000001", "40000000-0000-0000-0000-000000000001");
        let (button, button_c) = ("30000000-0000-0000-0000-000000000002", "40000000-0000-0000-0000-000000000002");
        let nested_in_main = "30000000-0000-0000-0000-000000000003";
        let copy = "30000000-0000-0000-0000-000000000004";
        let nested_in_copy = "30000000-0000-0000-0000-000000000005";
        let shapes = [
            (
                icon,
                serde_json::json!({"id":icon,"name":"Icon","type":"frame","width":16,"height":16,"componentId":icon_c,"mainInstance":true,"componentRoot":true}),
            ),
            (
                button,
                serde_json::json!({"id":button,"name":"Button","type":"frame","x":100,"width":80,"height":30,"shapes":[nested_in_main],
                "componentId":button_c,"mainInstance":true,"componentRoot":true}),
            ),
            (
                nested_in_main,
                serde_json::json!({"id":nested_in_main,"name":"Icon","type":"frame","parentId":button,"x":105,"y":5,"width":16,"height":16,
                "componentId":icon_c,"shapeRef":icon}),
            ),
            (
                copy,
                serde_json::json!({"id":copy,"name":"Button","type":"frame","x":300,"width":80,"height":30,"shapes":[nested_in_copy],
                "componentId":button_c,"shapeRef":button,"componentRoot":true}),
            ),
            (
                nested_in_copy,
                serde_json::json!({"id":nested_in_copy,"name":"Icon","type":"frame","parentId":copy,"x":305,"y":5,"width":16,"height":16,
                "componentId":icon_c,"shapeRef":nested_in_main}),
            ),
        ];
        let components = [
            (icon_c, serde_json::json!({"id":icon_c,"name":"Icon","mainInstanceId":icon})),
            (button_c, serde_json::json!({"id":button_c,"name":"Button","mainInstanceId":button})),
        ];
        let (doc, warnings) = import(&archive_with_components(&shapes, &components)).unwrap();
        assert!(!warnings.iter().any(|w| w.contains("kept independent") || w.contains("no native link")), "{warnings:?}");
        let by_penpot = |id: &str| doc.nodes().find(|n| n.plugin_data.get("penpot.id").map(String::as_str) == Some(id)).unwrap();
        let (icon, button, nested_main, copy, nested_copy) =
            (by_penpot(icon), by_penpot(button), by_penpot(nested_in_main), by_penpot(copy), by_penpot(nested_in_copy));
        assert!(matches!(nested_main.kind, NodeKind::Instance { component, .. } if component == icon.id));
        assert!(matches!(copy.kind, NodeKind::Instance { component, .. } if component == button.id));
        assert!(matches!(nested_copy.kind, NodeKind::Instance { component, .. } if component == icon.id));
        assert_eq!(nested_copy.source, Some(nested_main.id));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn penpot_transform_matrix_wins_and_children_are_not_transformed_twice() {
        let frame = "50000000-0000-0000-0000-000000000001";
        let child = "50000000-0000-0000-0000-000000000002";
        let mirror = serde_json::json!({"a":-1,"b":0,"c":0,"d":1,"e":0,"f":0});
        let shapes = [
            (
                frame,
                serde_json::json!({"id":frame,"name":"Mirrored","type":"frame","x":100,"y":50,"width":200,"height":100,
                "rotation":180,"transform":mirror,"shapes":[child]}),
            ),
            (
                child,
                serde_json::json!({"id":child,"name":"Dot","type":"rect","parentId":frame,"x":110,"y":60,"width":10,"height":10,
                "rotation":180,"transform":mirror}),
            ),
        ];
        let (doc, _) = import(&archive_with_shapes(&shapes)).unwrap();
        let frame = doc.nodes().find(|n| n.name == "Mirrored").unwrap();
        let child = doc.nodes().find(|n| n.name == "Dot").unwrap();
        let world = |t: Affine, x: f64, y: f64| t * figcraft_doc::geom::Point::new(x, y);
        assert_eq!(world(frame.transform, 0.0, 0.0), figcraft_doc::geom::Point::new(300.0, 50.0));
        let child_world = frame.transform * child.transform;
        assert_eq!(world(child_world, 0.0, 0.0), figcraft_doc::geom::Point::new(120.0, 60.0));
        assert_eq!(world(child_world, 10.0, 10.0), figcraft_doc::geom::Point::new(110.0, 70.0));
    }

    #[test]
    fn penpot_themes_become_modes_with_aliases_math_and_active_theme() {
        let tokens = serde_json::json!({
            "Core": {
                "gray": {"100": {"$value":"#F0F0F0","$type":"color"}, "900": {"$value":"rgb(17, 17, 17)","$type":"color"}},
                "space": {"base": {"$value":"8px","$type":"spacing"}, "lg": {"$value":"round({space.base} * 1.5 + max(2, 4))","$type":"spacing"}},
                "broken": {"$value":"{space.missing} * 2","$type":"spacing"},
                "uses-broken": {"$value":"{broken}","$type":"spacing"}
            },
            "Mode/Light": {"bg": {"$value":"{gray.100}","$type":"color"}},
            "Mode/Dark": {"bg": {"$value":"{gray.900}","$type":"color"}},
            "$themes": [
                {"id":"t-base","name":"Base","group":"Always","selectedTokenSets":{"Core":"enabled"}},
                {"id":"t-light","name":"Light","group":"Mode","selectedTokenSets":{"Mode/Light":"enabled"}},
                {"id":"t-dark","name":"Dark","group":"Mode","selectedTokenSets":{"Mode/Dark":"enabled"}}
            ],
            "$metadata": {"tokenSetOrder":["Core","Mode/Light","Mode/Dark"], "activeThemes":["Always/Base","Mode/Dark"], "activeSets":["Core","Mode/Dark"]}
        });
        let (doc, warnings) = import(&archive_with_design_system(tokens, None, None, None)).unwrap();
        assert!(!warnings.iter().any(|w| w.contains("Unrecognized")), "{warnings:?}");
        let vars = &doc.variables;
        let mode = vars.collections.iter().find(|c| c.name == "Mode").unwrap();
        assert_eq!(mode.modes.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Light", "Dark"]);
        let id = |name: &str| vars.variables.iter().find(|v| v.name.ends_with(&format!("/{name}"))).map(|v| v.id.clone());
        let bg = id("bg").unwrap();
        let in_mode = |m: usize| BTreeMap::from([(mode.id.clone(), mode.modes[m].id.clone())]);
        assert_eq!(vars.resolve(&bg, &in_mode(0)), Some(VariableValue::Color(Color::from_hex("#F0F0F0").unwrap())));
        assert_eq!(vars.resolve(&bg, &in_mode(1)), Some(VariableValue::Color(Color::from_rgb8_const(17, 17, 17))));
        assert_eq!(vars.resolve(&id("space.lg").unwrap(), &BTreeMap::new()), Some(VariableValue::Number(16.0)));
        assert!(id("broken").is_none() && id("uses-broken").is_none());
        let page = doc.node(doc.pages[0]).unwrap();
        assert_eq!(page.var_modes.get(&mode.id), Some(&mode.modes[1].id));
    }

    #[test]
    fn linked_instance_preserves_child_text_and_color_overrides() {
        let master = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let copy = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let component = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let main_text = "11111111-1111-1111-1111-111111111111";
        let copy_text = "22222222-2222-2222-2222-222222222222";
        let shapes = [
            (
                master,
                serde_json::json!({"id":master,"name":"Master", "type":"frame", "shapes":[main_text],"width":100,"height":40,"componentId":component,"mainInstance":true}),
            ),
            (
                main_text,
                serde_json::json!({"id":main_text,"name":"Label","type":"text","parentId":master,"width":90,"height":20,"content":{"children":[{"text":"Default"}]},"fills":[{"fillColor":"#000000"}]}),
            ),
            (
                copy,
                serde_json::json!({"id":copy,"name":"Button copy","type":"frame","x":180,"shapes":[copy_text],"width":100,"height":40,"componentId":component,"shapeRef":master}),
            ),
            (
                copy_text,
                serde_json::json!({"id":copy_text,"name":"Label","type":"text","parentId":copy,"shapeRef":main_text,"width":90,"height":20,"content":{"children":[{"text":"Changed"}]},"fills":[{"fillColor":"#FF0000"}]}),
            ),
        ];
        let components = [(component, serde_json::json!({"id":component,"mainInstanceId":master}))];
        let (doc, warnings) = import(&archive_with_components(&shapes, &components)).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let m = doc.nodes().find(|n| n.name == "Master").unwrap();
        let i = doc.nodes().find(|n| n.name == "Button copy").unwrap();
        assert!(matches!(i.kind, NodeKind::Instance { component, .. } if component == m.id));
        assert_eq!(i.source, Some(m.id));
        assert_ne!(i.transform, m.transform);
        let label = doc.nodes().find(|n| n.plugin_data.get("penpot.id").is_some_and(|v| v == copy_text)).unwrap();
        let orig = doc.nodes().find(|n| n.plugin_data.get("penpot.id").is_some_and(|v| v == main_text)).unwrap();
        assert_eq!(label.source, Some(orig.id));
        assert!(label.overrides.contains("characters"));
        assert!(label.overrides.contains("fills"));
        assert!(matches!(&label.kind, NodeKind::Text(t) if t.characters == "Changed"));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn detached_or_misordered_copies_are_not_linked() {
        let master = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let copy = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let component = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let main_child = "11111111-1111-1111-1111-111111111111";
        let extra = "22222222-2222-2222-2222-222222222222";
        let shapes = [
            (master, serde_json::json!({"id":master,"name":"Master","type":"frame","shapes":[main_child],"componentId":component})),
            (main_child, serde_json::json!({"id":main_child,"name":"Original","type":"rect","parentId":master})),
            (copy, serde_json::json!({"id":copy,"name":"Detached copy","type":"frame","shapes":[extra],"componentId":component,"shapeRef":master})),
            (extra, serde_json::json!({"id":extra,"name":"Inserted layer","type":"rect","parentId":copy})),
        ];
        let components = [(component, serde_json::json!({"id":component,"mainInstanceId":master}))];
        let (doc, warnings) = import(&archive_with_components(&shapes, &components)).unwrap();
        let copy_node = doc.nodes().find(|n| n.name == "Detached copy").unwrap();
        assert!(matches!(copy_node.kind, NodeKind::Frame));
        assert_eq!(copy_node.source, None);
        assert_eq!(copy_node.children.len(), 1);
        assert!(warnings.iter().any(|w| w.contains("kept independent")));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn unsupported_layouts_keep_positions_and_source_through_native_and_penpot_saves() {
        let frame = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let child = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        for (parent_props, child_props, warning) in [
            (serde_json::json!({"layoutFlexDir":"row-reverse"}), serde_json::json!({}), "reverse flow"),
            (serde_json::json!({"layoutFlexDir":"column-reverse"}), serde_json::json!({}), "reverse flow"),
            (serde_json::json!({"layoutAlignItems":"stretch"}), serde_json::json!({}), "stretch alignment"),
            (serde_json::json!({"layoutWrapType":"wrap","layoutAlignContent":"space-around"}), serde_json::json!({}), "wrapped-line distribution"),
            (serde_json::json!({"layoutWrapType":"wrap","layoutAlignContent":"space-evenly"}), serde_json::json!({}), "wrapped-line distribution"),
            (serde_json::json!({"layoutPadding":{"p1":-8,"p2":4,"p3":2,"p4":1}}), serde_json::json!({}), "negative padding"),
            (
                serde_json::json!({"layout":"grid","layoutGridColumns":[{"type":"percent","value":25}],"layoutGridRows":[{"type":"auto"}]}),
                serde_json::json!({}),
                "percentage tracks",
            ),
            (serde_json::json!({}), serde_json::json!({"layoutItemMargin":{"m1":12,"m2":0,"m3":-3,"m4":2}}), "item margins"),
            (serde_json::json!({}), serde_json::json!({"layoutItemAlignSelf":"end"}), "self-alignment"),
        ] {
            let mut parent = serde_json::json!({"id":frame,"type":"frame","name":"Layout","layout":"flex","width":200,"height":100,"shapes":[child]});
            parent.as_object_mut().unwrap().extend(parent_props.as_object().unwrap().clone());
            let mut item = serde_json::json!({"id":child,"type":"rect","name":"Child","parentId":frame,"x":17,"y":23,"width":40,"height":20});
            item.as_object_mut().unwrap().extend(child_props.as_object().unwrap().clone());
            let (doc, warnings) = import(&archive_with_shapes(&[(frame, parent), (child, item)])).unwrap();
            assert!(warnings.iter().any(|w| w.contains(warning)), "{warning}: {warnings:?}");
            let native = crate::save_bytes(&doc, None).unwrap();
            let saved = crate::load_bytes(&native).unwrap().0;
            let output = export(&saved).unwrap().0;
            let (back, _) = import(&output).unwrap();
            for doc in [&doc, &saved, &back] {
                let parent = doc.nodes().find(|n| n.name == "Layout").unwrap();
                let child = doc.nodes().find(|n| n.name == "Child").unwrap();
                assert!(parent.layout.is_none(), "{warning}");
                assert_eq!((child.transform.translation().x, child.transform.translation().y, child.width, child.height), (17.0, 23.0, 40.0, 20.0));
                for (node, props) in [(parent, &parent_props), (child, &child_props)] {
                    for (key, value) in props.as_object().unwrap() {
                        assert_eq!(node.plugin_data.get(&format!("penpot.source.{key}")), Some(&value.to_string()), "{warning}: {key}");
                    }
                }
            }
        }
    }

    #[test]
    fn an_unknown_flex_direction_is_kept_at_absolute_positions() {
        let frame = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let f = serde_json::json!({"id":frame,"type":"frame","name":"Diagonal","layout":"flex","layoutFlexDir":"diagonal"});
        let (doc, warnings) = import(&archive_with_shapes(&[(frame, f)])).unwrap();
        let n = doc.nodes().find(|n| n.name == "Diagonal").unwrap();
        assert!(n.layout.is_none());
        assert!(warnings.iter().any(|w| w.contains("Unknown Penpot flex direction")));
    }

    #[test]
    fn penpot_back_to_front_shapes_keep_visual_z_order() {
        let back = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let front = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let shapes = [
            (back, serde_json::json!({"id":back,"type":"rect","name":"Back"})),
            (front, serde_json::json!({"id":front,"type":"rect","name":"Front"})),
        ];
        let (doc, warnings) = import(&archive_with_shapes(&shapes)).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let page = doc.pages[0];
        let names = doc.node(page).unwrap().children.iter().map(|id| doc.node(*id).unwrap().name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["Back", "Front"]);
    }

    #[test]
    fn flex_children_flow_from_the_top_penpot_layer() {
        let frame = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let second = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let first = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let shapes = [
            (frame, serde_json::json!({"id":frame,"type":"frame","name":"Row","layout":"flex","layoutFlexDir":"row","shapes":[second,first]})),
            (second, serde_json::json!({"id":second,"parentId":frame,"type":"rect","name":"Second"})),
            (first, serde_json::json!({"id":first,"parentId":frame,"type":"rect","name":"First"})),
        ];
        let (doc, warnings) = import(&archive_with_shapes(&shapes)).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let row = doc.nodes().find(|n| n.name == "Row").unwrap();
        let names = row.children.iter().map(|id| doc.node(*id).unwrap().name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["First", "Second"]);
        assert!(row.layout.as_ref().unwrap().reverse_z);
    }

    #[test]
    fn multiline_paragraphs_remain_separated() {
        let frame = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let f = serde_json::json!({"id":frame,"type":"text","name":"Multi","content":{"type":"root","children":[{"type":"paragraph-set","children":[
            {"type":"paragraph","children":[{"text":"Alpha"}]},
            {"type":"paragraph","children":[{"text":"Beta"}]}
        ]}]}});
        let (doc, warnings) = import(&archive_with_shapes(&[(frame, f)])).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let n = doc.nodes().find(|n| n.name == "Multi").unwrap();
        match &n.kind {
            NodeKind::Text(t) => assert_eq!(t.characters, "Alpha\nBeta"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn unfilled_shapes_inherit_fills_from_groups_like_svg() {
        let ids = [
            "c4000000-0000-0000-0000-000000000001",
            "c4000000-0000-0000-0000-000000000002",
            "c4000000-0000-0000-0000-000000000003",
            "c4000000-0000-0000-0000-000000000004",
            "c4000000-0000-0000-0000-000000000005",
            "c4000000-0000-0000-0000-000000000006",
            "c4000000-0000-0000-0000-000000000007",
        ];
        let path = |id: &'static str, name: &str, parent: &str, attrs: Value| {
            (
                id,
                serde_json::json!({"id":id,"type":"path","name":name,"parentId":parent,"svgAttrs":attrs,"fills":[],
                "content":"M0 0 L10 0 L10 10 Z"}),
            )
        };
        let shapes = [
            (ids[0], serde_json::json!({"id":ids[0],"type":"group","name":"Icon","svgAttrs":{"width":"48"},"fills":[],"shapes":[ids[1], ids[2]]})),
            path(ids[1], "Outline", ids[0], serde_json::json!({})),
            path(ids[2], "Cutout", ids[0], serde_json::json!({"fill":"none"})),
            (ids[3], serde_json::json!({"id":ids[3],"type":"group","name":"Tinted","fills":[{"fillColor":"#A74FFF"}],"shapes":[ids[4]]})),
            path(ids[4], "Glyph", ids[3], serde_json::json!({})),
            (ids[5], serde_json::json!({"id":ids[5],"type":"frame","name":"Board","fills":[],"shapes":[ids[6]],"x":0,"y":0,"width":10,"height":10})),
            path(ids[6], "Loose", ids[5], serde_json::json!({})),
        ];
        let (doc, warnings) = import(&archive_with_shapes(&shapes)).unwrap();
        assert!(!warnings.iter().any(|w| w.contains("paint-server")));
        let fill = |name: &str| {
            doc.nodes().find(|n| n.name == name).unwrap().fills.iter().filter_map(Paint::solid_color).map(|c| c.to_rgba8()).collect::<Vec<_>>()
        };
        assert_eq!(fill("Outline"), [[0, 0, 0, 255]]);
        assert!(fill("Cutout").is_empty());
        assert_eq!(fill("Glyph"), [[0xA7, 0x4F, 0xFF, 255]]);
        assert!(fill("Loose").is_empty());
    }

    #[test]
    fn svg_style_fills_win_and_colorless_fills_do_not_inherit() {
        let group = "c6000000-0000-0000-0000-000000000001";
        let ids = ["c6000000-0000-0000-0000-000000000002", "c6000000-0000-0000-0000-000000000003", "c6000000-0000-0000-0000-000000000004"];
        let rect = |id: &'static str, name: &str, fills: Value, attrs: Value| {
            (
                id,
                serde_json::json!({"id":id,"type":"rect","name":name,"parentId":group,"x":0,"y":0,"width":10,"height":10,"fills":fills,"svgAttrs":attrs}),
            )
        };
        let shapes = [
            (group, serde_json::json!({"id":group,"type":"group","name":"Svg","svgAttrs":{"id":"layer1"},"fills":[],"shapes":ids})),
            rect(ids[0], "Gradient", serde_json::json!([{"fillOpacity":1}]), serde_json::json!({"style":{"fill":"url(#linearGradient927)"}})),
            rect(ids[1], "Styled", serde_json::json!([{"fillColor":"#FF0000"}]), serde_json::json!({"style":{"fill":"#00FF00"}})),
            rect(ids[2], "Colorless", serde_json::json!([{"fillOpacity":1}]), serde_json::json!({})),
        ];
        let (doc, warnings) = import(&archive_with_shapes(&shapes)).unwrap();
        let fill = |name: &str| {
            doc.nodes().find(|n| n.name == name).unwrap().fills.iter().filter_map(Paint::solid_color).map(|c| c.to_rgba8()).collect::<Vec<_>>()
        };
        assert!(fill("Gradient").is_empty());
        assert_eq!(fill("Styled"), [[0, 255, 0, 255]]);
        assert!(fill("Colorless").is_empty());
        assert!(warnings.iter().any(|w| w.contains("paint-server")));
    }

    #[test]
    fn svg_even_odd_fill_rule_keeps_holes_open() {
        let id = "ca000000-0000-0000-0000-000000000001";
        let ring = serde_json::json!({"id":id,"type":"path","name":"Ring","fills":[{"fillColor":"#000000"}],
            "svgAttrs":{"fillRule":"evenodd"},"content":"M0 0 L100 0 L100 100 L0 100 Z M25 25 L75 25 L75 75 L25 75 Z"});
        let (doc, _) = import(&archive_with_shapes(&[(id, ring)])).unwrap();
        let NodeKind::Vector { network } = &doc.nodes().find(|n| n.name == "Ring").unwrap().kind else { panic!("expected vector") };
        assert!(!network.regions.is_empty());
        assert!(network.regions.iter().all(|r| r.winding == figcraft_doc::WindingRule::EvenOdd));
    }

    #[test]
    fn svg_raw_boxes_are_never_painted() {
        let id = "c9000000-0000-0000-0000-000000000001";
        let raw = serde_json::json!({"id":id,"type":"svg-raw","name":"svg-text","x":0,"y":0,"width":1390,"height":1040,
            "fills":[{"fillColor":"#617670"}],"strokes":[{"strokeColor":"#000000","strokeWidth":1}],
            "content":{"tag":"text","attrs":{"x":"10","y":"20"},"content":["Title"]}});
        let (doc, _) = import(&archive_with_shapes(&[(id, raw)])).unwrap();
        let n = doc.nodes().find(|n| n.name == "svg-text").unwrap();
        assert!(n.fills.is_empty() && n.strokes.is_empty());
    }

    #[test]
    fn empty_penpot_paths_draw_nothing() {
        let id = "c7000000-0000-0000-0000-000000000001";
        let path = serde_json::json!({"id":id,"type":"path","name":"Stale","content":[],"fills":[{"fillColor":"#8445bc"}],
            "selrect":{"x":10,"y":10,"width":300,"height":100}});
        let (doc, warnings) = import(&archive_with_shapes(&[(id, path)])).unwrap();
        assert!(!warnings.iter().any(|w| w.contains("could not be decoded")), "{warnings:?}");
        let n = doc.nodes().find(|n| n.name == "Stale").unwrap();
        assert!(matches!(&n.kind, NodeKind::Vector { network } if network.vertices.is_empty()));
    }

    #[test]
    fn open_filled_subpaths_fill_like_svg_but_stroke_open() {
        let id = "c8000000-0000-0000-0000-000000000001";
        let path = serde_json::json!({"id":id,"type":"path","name":"Script","fills":[{"fillColor":"#262626"}],
            "strokes":[{"strokeColor":"#000000","strokeWidth":1}],
            "content":"M0 0 L100 0 L100 100 M200 0 L300 0 L300 100 L200 100 Z"});
        let (doc, _) = import(&archive_with_shapes(&[(id, path)])).unwrap();
        let NodeKind::Vector { network } = &doc.nodes().find(|n| n.name == "Script").unwrap().kind else { panic!("expected vector") };
        assert_eq!(network.segments.len(), 6);
        assert_eq!(network.regions.iter().map(|r| r.loops.len()).sum::<usize>(), 2);
        assert!(network.fill_path().contains(figcraft_doc::geom::Point::new(90.0, 10.0)));
        let unfilled = "c8000000-0000-0000-0000-000000000002";
        let line = serde_json::json!({"id":unfilled,"type":"path","name":"Line","strokes":[{"strokeColor":"#000000","strokeWidth":1}],
            "content":"M0 0 L100 0 L100 100"});
        let (doc, _) = import(&archive_with_shapes(&[(unfilled, line)])).unwrap();
        let NodeKind::Vector { network } = &doc.nodes().find(|n| n.name == "Line").unwrap().kind else { panic!("expected vector") };
        assert!(network.regions.is_empty());
    }

    #[test]
    fn first_penpot_paint_is_topmost() {
        let id = "c3000000-0000-0000-0000-000000000001";
        let rect = serde_json::json!({"id":id,"type":"rect","name":"Stack","x":0,"y":0,"width":10,"height":10,
            "fills":[{"fillColor":"#FF0000"},{"fillColor":"#00FF00"},{"fillColor":"#0000FF"}],
            "strokes":[{"strokeColor":"#111111"},{"strokeColor":"#222222"}]});
        let (doc, _) = import(&archive_with_shapes(&[(id, rect)])).unwrap();
        let n = doc.nodes().find(|n| n.name == "Stack").unwrap();
        let hex = |p: &[Paint]| p.iter().filter_map(Paint::solid_color).map(|c| c.to_rgba8()).collect::<Vec<_>>();
        assert_eq!(hex(&n.fills), [[0, 0, 255, 255], [0, 255, 0, 255], [255, 0, 0, 255]]);
        assert_eq!(hex(&n.strokes), [[0x22, 0x22, 0x22, 255], [0x11, 0x11, 0x11, 255]]);
    }

    #[test]
    fn paragraph_breaks_take_the_ended_paragraph_style() {
        let id = "cb000000-0000-0000-0000-000000000001";
        let para = |text: &str| serde_json::json!({"type":"paragraph","fontSize":"20","lineHeight":"1.5","children":[{"text":text}]});
        let text = serde_json::json!({"id":id,"type":"text","name":"Paragraphs","x":0,"y":0,"width":300,"height":100,
            "content":{"type":"root","children":[{"type":"paragraph-set","children":[para("One"), para(""), para("Two")]}]}});
        let (doc, _) = import(&archive_with_shapes(&[(id, text)])).unwrap();
        let NodeKind::Text(t) = &doc.nodes().find(|n| n.name == "Paragraphs").unwrap().kind else { panic!("expected text") };
        assert_eq!(t.characters, "One\n\nTwo");
        for i in 3..5 {
            assert_eq!(t.style_at(i).font_size, 20.0, "{i}");
            assert_eq!(t.style_at(i).line_height, LineHeight::Percent(150.0), "{i}");
        }
    }

    #[test]
    fn legacy_span_fill_colors_apply_when_fills_are_absent() {
        let id = "c5000000-0000-0000-0000-000000000001";
        let text = serde_json::json!({"id":id,"type":"text","name":"Legacy","x":0,"y":0,"width":200,"height":40,"fills":[],
        "content":{"type":"root","children":[{"type":"paragraph-set","children":[{"type":"paragraph","fillColor":"#ffffff","children":[
            {"text":"White ","fontSize":"20","fillColor":"#ffffff","fillOpacity":1},
            {"text":"Navy","fontSize":"20","fillColor":"#ffffff","fills":[{"fillColor":"#151035"}]}
        ]}]}]}});
        let (doc, _) = import(&archive_with_shapes(&[(id, text)])).unwrap();
        let NodeKind::Text(t) = &doc.nodes().find(|n| n.name == "Legacy").unwrap().kind else { panic!("expected text") };
        let color = |i: usize| t.style_at(i).fills.as_deref().and_then(|f| f.first()).and_then(Paint::solid_color).map(|c| c.to_rgba8());
        assert_eq!(color(0), Some([255, 255, 255, 255]));
        assert_eq!(color(7), Some([0x15, 0x10, 0x35, 255]));
    }

    #[test]
    fn named_font_weights_and_gradient_text_fills_survive() {
        let id = "c2000000-0000-0000-0000-000000000001";
        let gradient = serde_json::json!({"type":"linear","startX":0.5,"startY":0,"endX":0.5,"endY":1,"width":1,
            "stops":[{"color":"#25cbd9","offset":0,"opacity":1},{"color":"#00d1b8","offset":1,"opacity":1}]});
        let text = serde_json::json!({"id":id,"type":"text","name":"Title","x":0,"y":0,"width":200,"height":40,
        "content":{"type":"root","children":[{"type":"paragraph-set","children":[{"type":"paragraph","children":[
            {"text":"Bold ","fontFamily":"sourcesanspro","fontSize":"20","fontWeight":"bold","fills":[{"fillColor":"#151035"}]},
            {"text":"Teal","fontFamily":"sourcesanspro","fontSize":"20","fontWeight":"300","fills":[{"fillColorGradient":gradient}]}
        ]}]}]}});
        let (doc, _) = import(&archive_with_shapes(&[(id, text)])).unwrap();
        let NodeKind::Text(t) = &doc.nodes().find(|n| n.name == "Title").unwrap().kind else { panic!("expected text") };
        assert_eq!(t.style_at(0).font_style, "Bold");
        let teal = t.style_at(6);
        assert_eq!(teal.font_style, "Light");
        assert!(matches!(teal.fills.as_deref(), Some([p]) if matches!(p.kind, PaintKind::Gradient(_))));
    }

    #[test]
    fn near_identity_transforms_keep_boards_on_whole_pixels() {
        let id = "c1000000-0000-0000-0000-000000000001";
        let board = serde_json::json!({"id":id,"type":"frame","name":"Phone","x":1088,"y":1582,"width":384,"height":854,"shapes":[],
            "transform":{"a":0.9999999994569041,"b":1.745329248779787e-08,"c":1.745329237710893e-08,"d":1.000000000271156,"e":0,"f":0}});
        let (doc, _) = import(&archive_with_shapes(&[(id, board)])).unwrap();
        let n = doc.nodes().find(|n| n.name == "Phone").unwrap();
        assert_eq!(n.transform.as_coeffs(), [1.0, 0.0, 0.0, 1.0, 1088.0, 1582.0]);
        let id = "c1000000-0000-0000-0000-000000000002";
        let tilted = serde_json::json!({"id":id,"type":"rect","name":"Tilted","x":0,"y":0,"width":10,"height":10,"rotation":0.06,
            "transform":{"a":0.99999945,"b":0.00104719736,"c":-0.00104719736,"d":0.99999945,"e":0,"f":0}});
        let (doc, _) = import(&archive_with_shapes(&[(id, tilted)])).unwrap();
        assert_ne!(doc.nodes().find(|n| n.name == "Tilted").unwrap().transform.as_coeffs()[1], 0.0);
    }

    #[test]
    fn boards_clip_unless_show_content_is_set() {
        let shape = |id: &'static str, kind: &str, name: &str, show: Option<bool>| {
            let mut v = serde_json::json!({"id":id,"type":kind,"name":name,"x":0,"y":0,"width":10,"height":10,"shapes":[]});
            if let Some(show) = show {
                v["showContent"] = show.into();
            }
            (id, v)
        };
        let shapes = [
            shape("c0000000-0000-0000-0000-000000000001", "frame", "Default", None),
            shape("c0000000-0000-0000-0000-000000000002", "frame", "Shown", Some(true)),
            shape("c0000000-0000-0000-0000-000000000003", "frame", "Hidden", Some(false)),
            shape("c0000000-0000-0000-0000-000000000004", "group", "Group", None),
        ];
        let (doc, _) = import(&archive_with_shapes(&shapes)).unwrap();
        let clip = |name: &str| doc.nodes().find(|n| n.name == name).unwrap().clip;
        assert_eq!([clip("Default"), clip("Shown"), clip("Hidden"), clip("Group")], [true, false, true, false]);
    }

    #[test]
    fn stroke_styles_use_penpot_dash_arrays() {
        let rect = |id: &'static str, style: &str| {
            (
                id,
                serde_json::json!({"id":id,"type":"rect","name":style,"x":0,"y":0,"width":10,"height":10,
                "strokes":[{"strokeColor":"#000000","strokeWidth":2,"strokeStyle":style}]}),
            )
        };
        let shapes = [
            rect("d0000000-0000-0000-0000-000000000001", "dotted"),
            rect("d0000000-0000-0000-0000-000000000002", "dashed"),
            rect("d0000000-0000-0000-0000-000000000003", "mixed"),
            rect("d0000000-0000-0000-0000-000000000004", "solid"),
        ];
        let (doc, _) = import(&archive_with_shapes(&shapes)).unwrap();
        let stroke = |name: &str| doc.nodes().find(|n| n.name == name).unwrap().stroke.clone();
        assert_eq!(stroke("dotted").dashes, [0.0, 7.0]);
        assert_eq!((stroke("dotted").cap_start, stroke("dotted").cap_end), (StrokeCap::Round, StrokeCap::Round));
        assert_eq!(stroke("dashed").dashes, [12.0, 12.0]);
        assert_eq!(stroke("mixed").dashes, [7.0, 7.0, 3.0, 7.0]);
        assert!(stroke("solid").dashes.is_empty());
        let id = "d0000000-0000-0000-0000-000000000005";
        let arrow = serde_json::json!({"id":id,"type":"rect","name":"Arrow","x":0,"y":0,"width":10,"height":10,
            "strokes":[{"strokeColor":"#000000","strokeWidth":2,"strokeStyle":"dotted","strokeCapStart":"square","strokeCapEnd":"line-arrow"}]});
        let (doc, _) = import(&archive_with_shapes(&[(id, arrow)])).unwrap();
        let s = doc.nodes().find(|n| n.name == "Arrow").unwrap().stroke.clone();
        assert_eq!((s.cap_start, s.cap_end), (StrokeCap::Round, StrokeCap::LineArrow));
    }

    #[test]
    fn gradient_strokes_shadows_and_constraints_are_editable() {
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let gradient = serde_json::json!({
            "id":id,"type":"rect","name":"Gradient test","x":10,"y":20,"width":200,"height":100,
            "fills":[{"fillColor":"#00FF00","fillOpacity":0.7,"fillColorGradient":{
                "type":"linear","startX":0,"startY":0,"endX":1,"endY":1,
                "stops":[{"offset":0,"color":"#FF0000","opacity":1},{"offset":1,"color":"#0000FF","opacity":0.5}]
            }}],
            "strokes":[{"strokeColor":"#ABCD12","strokeOpacity":0.5,"strokeWidth":3,"strokeAlignment":"outer","strokeStyle":"dashed"}],
            "shadows":[{"style":"drop-shadow","color":{"color":"#123456","opacity":0.5},"offsetX":4,"offsetY":6,"blur":7,"spread":2}],
            "constraintsH":"right","constraintsV":"bottom","flipX":true
        });
        let (doc, warnings) = import(&archive_with_shapes(&[(id, gradient)])).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let n = doc.nodes().find(|n| n.name == "Gradient test").unwrap();
        assert!(matches!(&n.fills[0].kind, figcraft_doc::color::PaintKind::Gradient(_)));
        assert_eq!(n.strokes.len(), 1);
        assert!((n.strokes[0].opacity - 0.5).abs() < 0.01);
        assert_eq!(n.stroke.weight, 3.0);
        assert_eq!(n.stroke.align, StrokeAlign::Outside);
        assert!(!n.stroke.dashes.is_empty());
        assert_eq!(n.constraints.horizontal, HConstraint::Right);
        assert_eq!(n.constraints.vertical, VConstraint::Bottom);
        assert!(matches!(&n.effects[0].kind, EffectKind::DropShadow(_)));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn blur_modes_import_as_native_editable_effects() {
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let rectangle = serde_json::json!({"id":id,"name":"Blurred","type":"rect",
            "blur":{"type":"background-blur","value":8,"hidden":false}});
        let (doc, warnings) = import(&archive_with_shapes(&[(id, rectangle)])).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let n = doc.nodes().find(|n| n.name == "Blurred").unwrap();
        assert!(matches!(&n.effects[0].kind, EffectKind::BackgroundBlur(b) if b.radius == 8.0));
    }

    #[test]
    fn bezier_path_is_editable_vector_and_unsupported_paths_warn() {
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let vector = serde_json::json!({
            "id":id,"type":"path","name":"Curve","x":10,"y":20,"width":100,"height":100,
            "content":[
                {"command":"move-to","params":{"x":10,"y":20}},
                {"command":"line-to","params":{"x":110,"y":20}},
                {"command":"curve-to","params":{"c1x":110,"c1y":60,"c2x":80,"c2y":120,"x":10,"y":120}},
                {"command":"close-path"}
            ]
        });
        let (doc, warnings) = import(&archive_with_shapes(&[(id, vector.clone())])).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let n = doc.nodes().find(|n| n.name == "Curve").unwrap();
        match &n.kind {
            NodeKind::Vector { network } => {
                assert_eq!(network.regions.len(), 1);
                assert!(network.vertices.iter().any(|p| p.x == 100.0));
            }
            other => panic!("Expected editable vector, got {other:?}"),
        }
        let mut wrong = vector;
        // Elliptical arcs are supported in v5; use a genuinely unknown command
        // to exercise the fallback instead of relying on an obsolete expectation.
        wrong["content"][1]["command"] = "not-a-penpot-path-command".into();
        let (doc, warnings) = import(&archive_with_shapes(&[(id, wrong)])).unwrap();
        assert!(warnings.iter().any(|w| w.contains("Unsupported Penpot path command")));
        assert!(matches!(doc.nodes().find(|n| n.name == "Curve").unwrap().kind, NodeKind::Frame));
    }

    #[test]
    fn svg_string_path_content_from_penpot_2_10_is_editable_vector() {
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let vector = serde_json::json!({
            "id":id,"type":"path","name":"Icon","x":10,"y":20,"width":100,"height":100,
            "content":"M10.0,20.0L110.0,20.0C110.0,60.0,80.0,120.0,10.0,120.0Z"
        });
        let (doc, warnings) = import(&archive_with_shapes(&[(id, vector)])).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        match &doc.nodes().find(|n| n.name == "Icon").unwrap().kind {
            NodeKind::Vector { network } => {
                assert_eq!(network.regions.len(), 1);
                assert!(network.vertices.iter().any(|p| p.x == 100.0 && p.y == 0.0));
            }
            other => panic!("Expected editable vector, got {other:?}"),
        }
    }

    #[test]
    fn path_with_null_geometry_takes_its_box_from_selrect() {
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let line = serde_json::json!({
            "id":id,"type":"path","name":"Divider","x":null,"y":null,"width":null,"height":null,
            "selrect":{"x":380,"y":30,"width":0.01,"height":919},
            "content":"M380.0,30.0L380.0,949.0"
        });
        let (doc, warnings) = import(&archive_with_shapes(&[(id, line)])).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let n = doc.nodes().find(|n| n.name == "Divider").unwrap();
        assert_eq!((n.transform.translation().x, n.transform.translation().y, n.height), (380.0, 30.0, 919.0));
        match &n.kind {
            NodeKind::Vector { network } => assert!(network.vertices.iter().all(|p| p.x == 0.0)),
            other => panic!("Expected editable vector, got {other:?}"),
        }
    }

    #[test]
    fn styled_text_uses_unicode_character_offsets() {
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let value = serde_json::json!({
            "id":id,"type":"text","name":"Styled","x":0,"y":0,"width":100,"height":30,
            "content":{"type":"root","children":[
                {"text":"á😀","fontFamily":"Inter","fontSize":"20","fills":[{"fillColor":"#000000"}]},
                {"text":"B","fontFamily":"Inter","fontSize":"30","fontWeight":"700","fills":[{"fillColor":"#FF0000"}]}
            ]}
        });
        let (doc, warnings) = import(&archive_with_shapes(&[(id, value)])).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let n = doc.nodes().find(|n| n.name == "Styled").unwrap();
        if let NodeKind::Text(t) = &n.kind {
            assert_eq!(t.characters, "á😀B");
            assert_eq!(t.runs.len(), 2);
            assert_eq!((t.runs[0].start, t.runs[0].end), (0, 2));
            assert_eq!(t.runs[0].style.font_size, 20.0);
            assert_eq!((t.runs[1].start, t.runs[1].end), (2, 3));
            assert_eq!(t.runs[1].style.font_size, 30.0);
        } else {
            panic!("expected text");
        }
    }

    fn test_archive() -> Vec<u8> {
        let file = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
        let page = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
        let shape = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let buffer = Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(buffer);
        let entries = [
            ("manifest.json".to_string(), format!(r#"{{"type":"penpot/export-files","version":1,"files":[{{"id":"{file}","name":"Test"}}]}}"#)),
            (format!("files/{file}/pages/{page}.json"), format!(r#"{{"id":"{page}","name":"Canvas","index":0}}"#)),
            (format!("files/{file}/pages/{page}/{ROOT_ID}.json"), format!(r#"{{"id":"{ROOT_ID}","type":"frame","shapes":["{shape}"]}}"#)),
            (
                format!("files/{file}/pages/{page}/{shape}.json"),
                format!(
                    r##"{{"id":"{shape}","name":"Rect","type":"rect","x":10,"y":20,"width":100,"height":50,"fills":[{{"fillColor":"#AA3366"}}]}}"##
                ),
            ),
        ];
        for (path, data) in entries {
            z.start_file(path, SimpleFileOptions::default()).unwrap();
            z.write_all(data.as_bytes()).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    #[test]
    fn imports_editable_rectangle_and_page() {
        let (doc, notes) = import(&test_archive()).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(doc.name, "Test");
        assert_eq!(doc.pages.len(), 1);
        let rect = doc.nodes().find(|n| n.name == "Rect").unwrap();
        assert!(matches!(rect.kind, NodeKind::Rectangle));
        assert_eq!((rect.width, rect.height), (100.0, 50.0));
        assert_eq!(rect.fills.len(), 1);
    }

    /// Optional real-file regression test. Kept out of the repository: it may contain user assets.
    /// Run with FIGCRAFT_PENPOT_SAMPLE=/path/to/example.penpot cargo test -p figcraft-format penpot
    #[test]
    fn imports_sample_when_provided() {
        let Ok(path) = std::env::var("FIGCRAFT_PENPOT_SAMPLE") else {
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        let (doc, warnings) = import(&bytes).unwrap();
        assert!(!doc.pages.is_empty(), "{warnings:?}");
        assert!(doc.validate().is_empty(), "{warnings:?}");
    }

    #[test]
    fn external_component_reference_is_preserved_and_reported() {
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let unknown = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let frame = serde_json::json!({
            "id": id, "name": "External library button", "type": "frame",
            "componentId": unknown, "componentFile": "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee",
            "width": 200, "height": 50
        });
        let (doc, warnings) = import(&archive_with_shapes(&[(id, frame)])).unwrap();
        let node = doc.nodes().find(|n| n.name == "External library button").unwrap();
        assert!(matches!(node.kind, NodeKind::Frame));
        assert_eq!(node.plugin_data.get("penpot.componentLinkStatus").map(String::as_str), Some("not-linked"));
        assert!(warnings.iter().any(|w| w.contains("could not find their local masters")));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn rejects_unrelated_zip_and_corrupt_input() {
        assert!(import(b"not a zip").is_err());
        let mut zip = test_archive();
        zip.truncate(20);
        assert!(import(&zip).is_err());
    }
    fn archive_with_design_system(token_data: Value, typography: Option<Value>, color: Option<Value>, layer: Option<Value>) -> Vec<u8> {
        let file = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
        let page = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let typography_id = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let color_id = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let root_children = if layer.is_some() { vec![id] } else { Vec::new() };
        let mut entries = vec![
            (
                "manifest.json".to_string(),
                serde_json::json!({
                    "type":"penpot/export-files", "version":1, "files":[{"id":file,"name":"Design system"}]
                }),
            ),
            (format!("files/{file}/pages/{page}.json"), serde_json::json!({"id":page,"name":"Page","index":0})),
            (
                format!("files/{file}/pages/{page}/{ROOT_ID}.json"),
                serde_json::json!({
                    "id":ROOT_ID,"type":"frame","shapes":root_children
                }),
            ),
            (format!("files/{file}/tokens.json"), token_data),
        ];
        if let Some(value) = typography {
            entries.push((format!("files/{file}/typographies/{typography_id}.json"), value));
        }
        if let Some(value) = color {
            entries.push((format!("files/{file}/colors/{color_id}.json"), value));
        }
        if let Some(value) = layer {
            entries.push((format!("files/{file}/pages/{page}/{id}.json"), value));
        }
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (path, value) in entries {
            zip.start_file(path, SimpleFileOptions::default()).unwrap();
            zip.write_all(value.to_string().as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn v20_svg_paint_server_converts_simple_linear_gradient_without_flattening() {
        let id = "a0000000-0000-0000-0000-000000000001";
        let shape = serde_json::json!({
            "id":id,"type":"rect","name":"SVG gradient","width":100,"height":60,
            "svgAttrs":{"style":{"fill":"url(#brand-fill)"}},
            "svgDefs":[{"tag":"defs","content":[{"tag":"linearGradient","attrs":{"id":"brand-fill","x1":"0%","y1":"0%","x2":"100%","y2":"0%"},
                "content":[{"tag":"stop","attrs":{"offset":"0%","stop-color":"#112233"}},
                           {"tag":"stop","attrs":{"offset":"100%","stop-color":"#AABBCC","stop-opacity":"0.5"}}]}]}]
        });
        let (doc, warnings) = import(&archive_with_shapes(&[(id, shape)])).unwrap();
        let node = doc.nodes().find(|n| n.name == "SVG gradient").unwrap();
        let PaintKind::Gradient(g) = &node.fills[0].kind else { panic!("not a native gradient") };
        assert_eq!(g.kind, GradientKind::Linear);
        assert_eq!(g.stops.len(), 2);
        assert!((g.stops[1].color.a - 0.5).abs() < 0.01);
        assert!(!warnings.iter().any(|w| w.contains("paint-server")), "{warnings:?}");
        assert!(doc.validate().is_empty());
    }

    fn gradient_of<'a>(doc: &'a Document, name: &str) -> &'a Gradient {
        let node = doc.nodes().find(|n| n.name == name).unwrap();
        let Some(PaintKind::Gradient(g)) = node.fills.first().map(|p| &p.kind) else { panic!("{name}: no native gradient") };
        g
    }

    fn near(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn user_space_gradient_coordinates_follow_only_the_svg_transform_like_penpot() {
        let id = "a1000000-0000-0000-0000-000000000001";
        let path = serde_json::json!({"id":id,"type":"path","name":"Logo","content":"M2563 2341 L2585 2341 L2585 2385 L2563 2385 Z",
            "selrect":{"x":2563,"y":2341,"width":22,"height":44},"svgViewbox":{"x":0,"y":2,"width":11,"height":22},
            "svgTransform":{"a":1,"b":0,"c":0,"d":1,"e":2563,"f":2339},"svgAttrs":{"fill":"url(#paint2)"},
            "svgDefs":{"paint2":{"tag":"linearGradient","attrs":{"id":"paint2","x1":"8.25","y1":"2","x2":"26.95","y2":"39.4","gradientUnits":"userSpaceOnUse"},
                "content":[{"tag":"stop","attrs":{"stopColor":"#1CA7FD"}},{"tag":"stop","attrs":{"offset":"1","stopColor":"#148CFC"}}]}}});
        let (doc, warnings) = import(&archive_with_shapes(&[(id, path)])).unwrap();
        assert!(!warnings.iter().any(|w| w.contains("paint-server")), "{warnings:?}");
        let g = gradient_of(&doc, "Logo");
        assert!(near(g.from, (0.375, 0.0)) && near(g.to, (1.225, 0.85)), "{:?} {:?}", g.from, g.to);
        assert_eq!(g.stops[0].position, 0.0);
    }

    #[test]
    fn user_space_gradient_inherits_stops_through_href_with_css_stop_styles() {
        let id = "a1000000-0000-0000-0000-000000000002";
        let rect = serde_json::json!({"id":id,"type":"rect","name":"Cover","x":1,"y":0,"width":1389,"height":724,
            "selrect":{"x":1,"y":0,"width":1389,"height":724},"svgViewbox":{"x":0.28694993,"y":0.087415434,"width":367.53668,"height":191.9277},
            "svgTransform":null,"fills":[{"fillOpacity":1}],"svgAttrs":{"style":{"fill":"url(#linearGradient927)"}},
            "svgDefs":{
                "linearGradient927":{"tag":"linearGradient","attrs":{"gradientUnits":"userSpaceOnUse","id":"linearGradient927","xlinkHref":"#linearGradient925",
                    "x1":"0.28694993","y1":"96.05127","x2":"367.82364","y2":"96.05127"},"content":[]},
                "linearGradient925":{"tag":"linearGradient","attrs":{"id":"linearGradient925"},"content":[
                    {"tag":"stop","attrs":{"style":"stop-color:#f6901e;stop-opacity:1;","offset":"0"}},
                    {"tag":"stop","attrs":{"style":"stop-color:#972296;stop-opacity:0.5;","offset":"1"}}]}}});
        let (doc, _) = import(&archive_with_shapes(&[(id, rect)])).unwrap();
        let g = gradient_of(&doc, "Cover");
        assert!(near(g.from, (-0.00051, 0.13267)) && near(g.to, (0.26409, 0.13267)), "{:?} {:?}", g.from, g.to);
        assert_eq!(g.stops[0].color.to_rgba8(), [0xf6, 0x90, 0x1e, 255]);
        assert!((g.stops[1].color.a - 0.5).abs() < 0.01);
    }

    #[test]
    fn user_space_radial_gradient_maps_centre_and_radii() {
        let id = "a1000000-0000-0000-0000-000000000003";
        let rect = serde_json::json!({"id":id,"type":"rect","name":"Glow","x":0,"y":0,"width":100,"height":50,
            "svgAttrs":{"fill":"url(#glow)"},
            "svgDefs":{"glow":{"tag":"radialGradient","attrs":{"id":"glow","cx":"50","cy":"25","r":"25","gradientUnits":"userSpaceOnUse"},
                "content":[{"tag":"stop","attrs":{"stop-color":"#FFFFFF"}},{"tag":"stop","attrs":{"offset":"1","stop-color":"#000000"}}]}}});
        let (doc, _) = import(&archive_with_shapes(&[(id, rect)])).unwrap();
        let g = gradient_of(&doc, "Glow");
        assert_eq!(g.kind, GradientKind::Radial);
        assert!(near(g.from, (0.5, 0.5)) && near(g.to, (0.75, 0.5)) && near(g.width, (0.5, 1.0)), "{:?} {:?} {:?}", g.from, g.to, g.width);
    }

    #[test]
    fn same_gradient_id_in_two_shapes_keeps_each_shapes_own_colours() {
        let shape = |id: &'static str, name: &str, color: &str| {
            (
                id,
                serde_json::json!({"id":id,"type":"rect","name":name,"x":0,"y":0,"width":10,"height":10,"svgAttrs":{"fill":"url(#paint0)"},
                "svgDefs":{"paint0":{"tag":"linearGradient","attrs":{"id":"paint0","x1":"0","y1":"0","x2":"10","y2":"0","gradientUnits":"userSpaceOnUse"},
                    "content":[{"tag":"stop","attrs":{"stopColor":color}},{"tag":"stop","attrs":{"offset":"1","stopColor":color}}]}}}),
            )
        };
        let shapes =
            [shape("a1000000-0000-0000-0000-000000000004", "Red", "#FF0000"), shape("a1000000-0000-0000-0000-000000000005", "Blue", "#0000FF")];
        let (doc, _) = import(&archive_with_shapes(&shapes)).unwrap();
        assert_eq!(gradient_of(&doc, "Red").stops[0].color.to_rgba8(), [255, 0, 0, 255]);
        assert_eq!(gradient_of(&doc, "Blue").stops[0].color.to_rgba8(), [0, 0, 255, 255]);
    }

    #[test]
    fn v20_unsupported_svg_gradient_does_not_approximate_or_erase_source() {
        let id = "a0000000-0000-0000-0000-000000000002";
        let shape = serde_json::json!({"id":id,"type":"rect","name":"Transformed",
            "svgAttrs":{"style":{"fill":"url(#tx)"}},
            "svgDefs":[{"tag":"radialGradient","attrs":{"id":"tx","fx":"0.2"},
                "content":[{"tag":"stop","attrs":{"stop-color":"#FF0000"}}]}]});
        let (doc, warnings) = import(&archive_with_shapes(&[(id, shape)])).unwrap();
        let node = doc.nodes().find(|n| n.name == "Transformed").unwrap();
        assert!(node.fills.is_empty());
        assert!(warnings.iter().any(|w| w.contains("paint-server")), "{warnings:?}");
        assert!(node.plugin_data.contains_key("penpot.source.svgAttrs"));
    }

    #[test]
    fn v20_compound_tokens_are_reusable_and_survive_round_trip() {
        let tokens = serde_json::json!({"sets":{"Core":{"tokens":{
            "title":{"type":"typography","value":{"fontFamily":"Spicy Rice","fontSize":"24px","fontWeight":400,"lineHeight":"32px","letterSpacing":"1px"}},
            "elevation":{"type":"boxShadow","value":[{"color":"#112233","x":"2px","y":"4px","blur":"6px","spread":"1px"}]},
            "unsafe":{"type":"boxShadow","value":{"color":"#112233","x":"{spacing.sm}","y":"0px","blur":"1px"}}
        }}}});
        let (doc, warnings) = import(&archive_with_design_system(tokens, None, None, None)).unwrap();
        assert_eq!(doc.styles.len(), 2);
        assert!(doc.styles.iter().any(|s| s.name == "Penpot tokens/Core/title"
            && matches!(&s.kind, StyleKind::Text { style }
            if style.font_family == "Spicy Rice" && style.font_size == 24.0 && style.line_height == LineHeight::Pixels(32.0))));
        assert!(doc.styles.iter().any(|s| s.name == "Penpot tokens/Core/elevation"
            && matches!(&s.kind, StyleKind::Effect { effects }
            if effects.len() == 1 && matches!(&effects[0].kind, EffectKind::DropShadow(sh) if sh.offset == (2.0,4.0) && sh.radius == 6.0))));
        assert!(warnings.iter().any(|w| w.contains("compound tokens could not")), "{warnings:?}");
        let saved = crate::save_bytes(&doc, None).unwrap();
        let (opened, issues) = crate::load_bytes(&saved).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(opened.styles, doc.styles);
    }

    #[test]
    fn penpot_v11_imports_scalar_tokens_into_native_variables_and_preserves_theme_source() {
        let tokens = serde_json::json!({
            "sets":{"Core":{"tokens":{
                "color":{"primary":{"type":"color","value":"#0066CC"}},
                "spacing":{"sm":{"type":"dimension","value":"8px"}},
                "copy":{"cta":{"type":"string","value":"Buy"}},
                "flags":{"enabled":{"type":"boolean","value":true}},
                "aliases":{"secondary":{"type":"color","value":"{color.primary}"}}
            }}},
            "themes":{"Light":{"sets":["Core"]}},"activeThemes":["Light"]
        });
        let (doc, warnings) = import(&archive_with_design_system(tokens.clone(), None, None, None)).unwrap();
        assert_eq!(doc.variables.collections.len(), 1);
        assert_eq!(doc.variables.variables.len(), 5);
        let values = &doc.variables;
        assert!(matches!(values.resolve(&values.get("Core/color.primary").unwrap().id, &BTreeMap::new()), Some(VariableValue::Color(_))));
        assert_eq!(values.resolve(&values.get("Core/spacing.sm").unwrap().id, &BTreeMap::new()), Some(VariableValue::Number(8.0)));
        assert!(
            matches!(values.resolve(&values.get("Core/copy.cta").unwrap().id, &BTreeMap::new()), Some(VariableValue::String(ref s)) if s == "Buy")
        );
        assert_eq!(values.resolve(&values.get("Core/flags.enabled").unwrap().id, &BTreeMap::new()), Some(VariableValue::Boolean(true)));
        assert!(warnings.iter().any(|s| s.contains("themes")));
        assert!(matches!(values.resolve(&values.get("Core/aliases.secondary").unwrap().id, &BTreeMap::new()), Some(VariableValue::Color(_))));
        assert!(!warnings.iter().any(|s| s.contains("unsupported or unresolved")));
        let page = doc.node(doc.first_page().unwrap()).unwrap();
        assert_eq!(
            page.plugin_data.get("penpot.tokens.aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").and_then(|s| serde_json::from_str::<Value>(s).ok()),
            Some(tokens)
        );
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn penpot_v11_imports_named_text_and_color_styles() {
        let typography_id = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let color_id = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let typ = serde_json::json!({"id":typography_id,"name":"Heading","path":"Brand",
            "fontFamily":"Inter","fontSize":"24","fontWeight":"700","fontStyle":"normal"});
        let color = serde_json::json!({"id":color_id,"name":"Primary","path":"Brand",
            "color":"#224466","opacity":0.5});
        let (doc, warnings) = import(&archive_with_design_system(serde_json::json!({"sets":{}}), Some(typ), Some(color), None)).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(doc.styles.len(), 2);
        assert!(
            doc.styles.iter().any(|s| s.name == "Brand/Heading"
                && matches!(&s.kind, StyleKind::Text { style } if style.font_family == "Inter" && style.font_size == 24.0))
        );
        assert!(doc.styles.iter().any(|s| s.name == "Brand/Primary" && matches!(s.kind, StyleKind::Paint { .. })));
        assert_eq!(doc.document_colors.len(), 1);
    }

    #[test]
    fn penpot_v11_variant_fields_are_preserved_without_false_native_link() {
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let shape = serde_json::json!({
            "id":id,"name":"Button / Disabled","type":"frame","width":100,"height":32,
            "variantId":"ffffffff-ffff-ffff-ffff-ffffffffffff",
            "variantProperties":{"State":"Disabled"},
        });
        let (doc, warnings) = import(&archive_with_design_system(serde_json::json!({"sets":{}}), None, None, Some(shape))).unwrap();
        let button = doc.nodes().find(|node| node.name == "Button / Disabled").unwrap();
        assert!(matches!(button.kind, NodeKind::Frame));
        assert_eq!(button.plugin_data.get("penpot.variantId").map(String::as_str), Some("ffffffff-ffff-ffff-ffff-ffffffffffff"));
        assert!(button.plugin_data.contains_key("penpot.source.variantProperties"));
        assert!(warnings.iter().any(|s| s.contains("variant layers")));
    }

    #[test]
    fn penpot_v14_binds_matching_color_and_size_tokens_and_round_trips() {
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let tokens = serde_json::json!({"sets":{"Core":{"tokens":{
            "palette":{"primary":{"type":"color","value":"#336699"},
                "button":{"type":"color","value":"{palette.primary}"}},
            "size":{"width":{"type":"dimension","value":"120px"},
                "height":{"type":"number","value":60}}
        }}}});
        let layer = serde_json::json!({"id":id,"name":"Token button","type":"rect",
            "x":5,"y":8,"width":120,"height":60,
            "fills":[{"fillColor":"#336699"}],
            "strokes":[{"strokeColor":"#336699","strokeWidth":1}],
            "appliedTokens":{
                "fill":"palette.button","strokeColor":"palette.primary",
                "width":"size.width","height":"size.height"
            }
        });
        let (doc, warnings) = import(&archive_with_design_system(tokens, None, None, Some(layer))).unwrap();
        let n = doc.nodes().find(|n| n.name == "Token button").unwrap();
        assert_eq!(n.bound.len(), 4);
        let color = doc.variables.get("Core/palette.button").unwrap();
        assert_eq!(n.bound.get("fills/0/color"), Some(&color.id));
        assert_eq!(n.bound.get("width"), doc.variables.get("Core/size.width").map(|v| &v.id));
        assert_eq!(n.bound.get("height"), doc.variables.get("Core/size.height").map(|v| &v.id));
        assert!(warnings.iter().any(|w| w.contains("Linked 4 Penpot token")), "{warnings:?}");
        let bytes = crate::save_bytes(&doc, None).unwrap();
        let (restored, issues) = crate::load_bytes(&bytes).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        let restored_node = restored.nodes().find(|n| n.name == "Token button").unwrap();
        assert_eq!(restored_node.bound, n.bound);
        assert!(restored_node.plugin_data.contains_key("penpot.source.appliedTokens"));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn bindings_accept_short_names_in_nested_token_sets() {
        let tokens = serde_json::json!({"sets":{"Mode/Light":{"tokens":{
            "palette/bg":{"type":"color","value":"#336699"},
            "width":{"type":"number","value":120}
        }}}});
        let layer = serde_json::json!({"id":"cccccccc-cccc-cccc-cccc-cccccccccccc", "name":"Nested token set", "type":"rect",
            "width":120,"height":60,"fills":[{"fillColor":"#336699"}],
            "appliedTokens":{"fill":"palette/bg","width":"Mode/Light/width"}});
        let (doc, warnings) = import(&archive_with_design_system(tokens, None, None, Some(layer))).unwrap();
        let node = doc.nodes().find(|n| n.name == "Nested token set").unwrap();
        assert_eq!(node.bound.get("fills/0/color"), doc.variables.get("Mode/Light/palette/bg").map(|v| &v.id));
        assert_eq!(node.bound.get("width"), doc.variables.get("Mode/Light/width").map(|v| &v.id));
        assert_eq!(node.bound.len(), 2, "{warnings:?}");
    }

    #[test]
    fn penpot_v14_refuses_ambiguous_sets_and_different_resolved_values() {
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let tokens = serde_json::json!({"sets":{
            "Core":{"tokens":{"brand":{"type":"color","value":"#336699"},
                "unique":{"type":"color","value":"#000000"}}},
            "Alternate":{"tokens":{"brand":{"type":"color","value":"#336699"}}}
        }});
        let layer = serde_json::json!({"id":id,"name":"Do not relink","type":"rect",
            "x":1,"y":2,"width":80,"height":40,
            "fills":[{"fillColor":"#336699"}],
            "appliedTokens":{"fill":"brand","strokeColor":"unique", "opacity":"brand"}
        });
        let (doc, warnings) = import(&archive_with_design_system(tokens, None, None, Some(layer))).unwrap();
        let n = doc.nodes().find(|n| n.name == "Do not relink").unwrap();
        assert!(n.bound.is_empty());
        assert!(n.plugin_data.contains_key("penpot.source.appliedTokens"));
        assert!(warnings.iter().any(|w| w.contains("could not be safely linked")), "{warnings:?}");
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn penpot_v14_bindings_reject_color_mismatch_and_accept_explicit_set_name() {
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let tokens = serde_json::json!({"sets":{
            "Core":{"tokens":{"brand":{"type":"color","value":"#336699"},
                "w":{"type":"number","value":125}}},
            "Alternate":{"tokens":{"brand":{"type":"color","value":"#000000"}}}
        }});
        let layer = serde_json::json!({"id":id,"name":"Explicit set","type":"rect",
            "x":1,"y":2,"width":50,"height":40,
            "fills":[{"fillColor":"#336699"}],
            "appliedTokens":{"fill":"Core/brand", "width":"w"}
        });
        let (doc, warnings) = import(&archive_with_design_system(tokens, None, None, Some(layer))).unwrap();
        let n = doc.nodes().find(|n| n.name == "Explicit set").unwrap();
        assert_eq!(n.bound.len(), 1);
        assert_eq!(n.bound.get("fills/0/color"), doc.variables.get("Core/brand").map(|v| &v.id));
        assert!(!n.bound.contains_key("width"));
        assert!(warnings.iter().any(|w| w.contains("could not be safely linked")), "{warnings:?}");
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn penpot_v13_alias_chains_are_editable_and_survive_round_trip() {
        let tokens = serde_json::json!({"sets":{"Core":{"tokens":{
            "palette":{
                "base":{"type":"color","value":"#336699"},
                "middle":{"type":"color","value":"{palette.base}"},
                "accent":{"type":"color","value":"{palette.middle}"}
            },
            "space":{
                "small":{"type":"dimension","value":"12px"},
                "copy":{"type":"dimension","value":"{space.small}"}
            },
            "label":{"type":"string","value":"Button"},
            "labelAlias":{"type":"string","value":"{label}"}
        }}}});
        let (doc, warnings) = import(&archive_with_design_system(tokens, None, None, None)).unwrap();
        assert_eq!(doc.variables.variables.len(), 7);
        let v = &doc.variables;
        assert!(matches!(v.get("Core/palette.middle").unwrap().values.values().next(), Some(VariableValue::Alias(_))));
        assert!(matches!(v.resolve(&v.get("Core/palette.accent").unwrap().id, &BTreeMap::new()), Some(VariableValue::Color(_))));
        assert_eq!(v.resolve(&v.get("Core/space.copy").unwrap().id, &BTreeMap::new()), Some(VariableValue::Number(12.0)));
        assert_eq!(v.resolve(&v.get("Core/labelAlias").unwrap().id, &BTreeMap::new()), Some(VariableValue::String("Button".into())));
        assert!(!warnings.iter().any(|w| w.contains("unsupported or unresolved")), "{warnings:?}");
        let saved = crate::save_bytes(&doc, None).unwrap();
        let (reloaded, issues) = crate::load_bytes(&saved).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(reloaded.variables, doc.variables);
    }

    #[test]
    fn penpot_v13_rejects_bad_aliases_without_creating_live_broken_references() {
        let tokens = serde_json::json!({"sets":{"Core":{"tokens":{
            "base":{"type":"color","value":"#123456"},
            "unknown":{"type":"color","value":"{not.present}"},
            "cycleA":{"type":"color","value":"{cycleB}"},
            "cycleB":{"type":"color","value":"{cycleA}"},
            "number":{"type":"number","value":5},
            "wrongType":{"type":"color","value":"{number}"}
        }}}});
        let (doc, warnings) = import(&archive_with_design_system(tokens, None, None, None)).unwrap();
        assert_eq!(doc.variables.variables.len(), 2);
        assert!(doc.variables.get("Core/unknown").is_none());
        assert!(doc.variables.get("Core/cycleA").is_none());
        assert!(doc.variables.get("Core/wrongType").is_none());
        assert!(warnings.iter().any(|w| w.contains("unsupported or unresolved")));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn penpot_v13_recovers_orphaned_subtree_as_editable_layers() {
        let missing = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let parent = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let child = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let items = [
            (
                parent,
                serde_json::json!({"id":parent,"name":"Detached group","type":"group",
                "parentId":missing,"x":100,"y":75,"width":120,"height":90,"shapes":[child]}),
            ),
            (
                child,
                serde_json::json!({"id":child,"name":"Recovered child","type":"rect",
                "parentId":parent,"x":110,"y":85,"width":40,"height":20}),
            ),
        ];
        let (doc, warnings) = import(&archive_with_shapes(&items)).unwrap();
        let parent_node = doc.nodes().find(|n| n.name == "Detached group").unwrap();
        let child_node = doc.nodes().find(|n| n.name == "Recovered child").unwrap();
        assert_eq!(parent_node.parent, doc.first_page());
        assert_eq!(child_node.parent, Some(parent_node.id));
        assert!(matches!(child_node.kind, NodeKind::Rectangle));
        assert!(warnings.iter().any(|w| w.contains("orphan layer")));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn penpot_v13_recovers_cyclic_parent_graph_without_recursing_forever() {
        let a = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let b = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let items = [
            (
                a,
                serde_json::json!({"id":a,"name":"A","type":"group","parentId":b,
                "width":80,"height":80,"shapes":[b]}),
            ),
            (
                b,
                serde_json::json!({"id":b,"name":"B","type":"rect","parentId":a,
                "width":20,"height":20,"shapes":[a]}),
            ),
        ];
        let (doc, warnings) = import(&archive_with_shapes(&items)).unwrap();
        assert_eq!(doc.nodes().filter(|n| n.name == "A" || n.name == "B").count(), 2);
        assert!(warnings.iter().any(|w| w.contains("cyclic layer graph")));
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn penpot_v12_round_trip_preserves_styles_variables_and_original_theme_json() {
        let id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let typ_id = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let tokens = serde_json::json!({
            "sets":{"Core":{"tokens":{"brand":{"type":"color","value":"#224466"}}}},
            "themes":{"Light":{"sets":["Core"]}}, "activeThemes":["Light"]
        });
        let original = archive_with_design_system(
            tokens.clone(),
            Some(serde_json::json!({"id":typ_id,"name":"Body","fontFamily":"Inter","fontSize":16})),
            None,
            Some(serde_json::json!({"id":id,"type":"frame","name":"Variant / Hover", "width":100, "height":40,
                "variantProperties":{"State":"Hover"}})),
        );
        let (before, _) = import(&original).unwrap();
        let stored = crate::save_bytes(&before, None).unwrap();
        let (after, issues) = crate::load_bytes(&stored).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(before.node_count(), after.node_count());
        assert_eq!(before.pages.len(), after.pages.len());
        assert_eq!(before.styles, after.styles);
        assert_eq!(before.variables, after.variables);
        let page = after.node(after.first_page().unwrap()).unwrap();
        let saved = page.plugin_data.get("penpot.tokens.aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap();
        assert_eq!(serde_json::from_str::<Value>(saved).unwrap(), tokens);
        let variant = after.nodes().find(|node| node.name == "Variant / Hover").unwrap();
        assert_eq!(variant.plugin_data.get("penpot.source.variantProperties").map(String::as_str), Some(r#"{"State":"Hover"}"#));
        assert!(after.validate().is_empty());
    }

    #[test]
    fn penpot_v12_rejects_duplicate_and_traversal_member_paths() {
        let original = archive_with_design_system(serde_json::json!({"sets":{}}), None, None, None);
        // Deliberate duplicate manifest; accepting this could result in validating
        // one manifest and resolving a different one by name.
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, payload) in [("manifest.json", &original[..1]), ("manifest.jsoX", &original[..1])] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(payload).unwrap();
        }
        let mut bytes = zip.finish().unwrap().into_inner();
        let twin = b"manifest.jsoX";
        let mut at = 0;
        while let Some(i) = bytes[at..].windows(twin.len()).position(|w| w == twin) {
            bytes[at + i + twin.len() - 1] = b'n';
            at += i + twin.len();
        }
        assert!(import(&bytes).is_err());

        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("../manifest.json", SimpleFileOptions::default()).unwrap();
        zip.write_all(b"{}").unwrap();
        assert!(import(&zip.finish().unwrap().into_inner()).is_err());
    }

    #[test]
    fn penpot_token_expressions_have_a_bounded_recursion_depth() {
        let parse = |text: &str| {
            let mut resolve: fn(&str, usize) -> Option<f64> = |_, _| None;
            TokenExpr { text: text.as_bytes(), at: 0, nesting: 0, resolve: &mut resolve }.parse()
        };
        assert_eq!(parse("-(-(-2))"), Some(-2.0));
        assert_eq!(parse("max(1, min(4, 2))"), Some(2.0));
        // All of these used to recurse until a Rust stack overflow aborted the process.
        for hostile in
            ["-".repeat(100_000) + "1", "(".repeat(100_000) + "1" + &")".repeat(100_000), "min(".repeat(100_000) + "1" + &")".repeat(100_000)]
        {
            assert_eq!(parse(&hostile), None);
        }
        let valid = "(".repeat(MAX_TOKEN_EXPR_NESTING - 1) + "1" + &")".repeat(MAX_TOKEN_EXPR_NESTING - 1);
        assert_eq!(parse(&valid), Some(1.0));
        let over = "(".repeat(MAX_TOKEN_EXPR_NESTING) + "1" + &")".repeat(MAX_TOKEN_EXPR_NESTING);
        assert_eq!(parse(&over), None);
    }

    #[test]
    fn penpot_token_aliases_share_the_expression_nesting_budget() {
        // A nested expression can resolve an alias whose own expression is nested.
        // Counting each string separately would allow alias chains to bypass the limit.
        let first = "(".repeat(50) + "{second}" + &")".repeat(50);
        let second = "(".repeat(50) + "1" + &")".repeat(50);
        let values = serde_json::json!({
            "first": {"$value": first},
            "second": {"$value": second},
            "short": {"$value": "1 + {second}"}
        });
        let mut set = BTreeMap::new();
        for name in ["first", "second", "short"] {
            set.insert(name.to_owned(), &values[name]);
        }
        let tokens = BTreeMap::from([("Core", set)]);
        let mut budget = TokenResolutionBudget { remaining: MAX_TOKEN_RESOLUTION_WORK };
        assert_eq!(token_number("first", &["Core"], &tokens, 0, 0, &mut budget), None);
        assert_eq!(token_number("short", &["Core"], &tokens, 0, 0, &mut budget), Some(2.0));
    }

    #[test]
    fn penpot_token_branching_aliases_exhaust_shared_work_budget() {
        // Repeated references multiply the number of evaluations, even with shallow nesting.
        let source: Vec<Value> = (0..22)
            .map(|i| {
                let value = if i == 0 { "0".to_owned() } else { format!("{{t{}}} + {{t{}}}", i - 1, i - 1) };
                serde_json::json!({"$type": "number", "$value": value})
            })
            .collect();
        let set = source.iter().enumerate().map(|(i, v)| (format!("t{i}"), v)).collect();
        let tokens = BTreeMap::from([("Core", set)]);
        let mut budget = TokenResolutionBudget { remaining: 1024 };
        assert_eq!(token_number("t21", &["Core"], &tokens, 0, 0, &mut budget), None);
        assert!(budget.remaining < 1024);
        // The same exhausted budget must also refuse a subsequent, otherwise valid token.
        assert_eq!(token_number("t0", &["Core"], &tokens, 0, 0, &mut budget), None);
    }

    #[test]
    fn penpot_token_work_budget_is_shared_between_independent_tokens() {
        let values = serde_json::json!({
            "a": {"$type": "number", "$value": "1 + 2"},
            "b": {"$type": "number", "$value": "3 + 4"},
            "c": {"$type": "number", "$value": "5 + 6"}
        });
        let tokens = BTreeMap::from([(
            "Core",
            BTreeMap::from([("a".to_owned(), &values["a"]), ("b".to_owned(), &values["b"]), ("c".to_owned(), &values["c"])]),
        )]);

        // Measure each token independently, then apply exactly their combined cost
        // to one shared budget. This also catches changes in accounting rules.
        let initial = 100usize;
        let mut a_budget = TokenResolutionBudget { remaining: initial };
        assert_eq!(token_number("a", &["Core"], &tokens, 0, 0, &mut a_budget), Some(3.0));
        let a_cost = initial - a_budget.remaining;
        let mut b_budget = TokenResolutionBudget { remaining: initial };
        assert_eq!(token_number("b", &["Core"], &tokens, 0, 0, &mut b_budget), Some(7.0));
        let b_cost = initial - b_budget.remaining;
        assert!(a_cost > 0 && b_cost > 0);
        let mut budget = TokenResolutionBudget { remaining: a_cost + b_cost };
        assert_eq!(token_number("a", &["Core"], &tokens, 0, 0, &mut budget), Some(3.0));
        assert_eq!(token_number("b", &["Core"], &tokens, 0, 0, &mut budget), Some(7.0));
        assert_eq!(token_number("c", &["Core"], &tokens, 0, 0, &mut budget), None);
    }

    #[test]
    fn penpot_v12_malformed_token_values_are_safe_and_preserve_source() {
        let tokens = serde_json::json!({"sets":{"Core":{"tokens":{
            "bad":{"type":"dimension","value":"NaNpx"},
            "huge":{"type":"dimension","value":1e30},
            "unknown":{"type":"shadow","value":{"x":3}},
            "ok":{"type":"number","value":12}
        }}}});
        let (doc, warnings) = import(&archive_with_design_system(tokens.clone(), None, None, None)).unwrap();
        assert_eq!(doc.variables.variables.len(), 1);
        assert!(warnings.iter().any(|w| w.contains("unsupported or unresolved")));
        let page = doc.node(doc.first_page().unwrap()).unwrap();
        let source = page.plugin_data.get("penpot.tokens.aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap();
        assert_eq!(serde_json::from_str::<Value>(source).unwrap(), tokens);
        assert!(doc.validate().is_empty());
    }

    #[test]
    fn penpot_v12_user_sample_can_be_imported_saved_and_reopened() {
        let Ok(path) = std::env::var("FIGCRAFT_PENPOT_SAMPLE") else {
            return;
        };
        let data = std::fs::read(&path).unwrap();
        let (doc, warnings) = import(&data).unwrap();
        assert!(!doc.pages.is_empty());
        assert!(doc.validate().is_empty(), "{warnings:?}");
        let saved = crate::save_bytes(&doc, None).unwrap();
        let (opened, issues) = crate::load_bytes(&saved).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(opened.pages.len(), doc.pages.len());
        assert_eq!(opened.node_count(), doc.node_count());
        assert_eq!(opened.variables, doc.variables);
        assert_eq!(opened.styles, doc.styles);
        for n in doc.nodes().filter(|n| n.plugin_data.contains_key("penpot.id")) {
            assert!(opened.nodes().any(|o| o.plugin_data.get("penpot.id") == n.plugin_data.get("penpot.id")));
        }
    }

    // --- Prototypes

    use figcraft_doc::proto::Action;

    /// Two frames and a button that navigates from one to the other.
    fn prototype_shapes() -> Vec<(&'static str, Value)> {
        let first = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let second = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        let button = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        vec![
            (first, serde_json::json!({"id":first,"type":"frame","name":"Home","x":0,"y":0,"width":200,"height":100})),
            (second, serde_json::json!({"id":second,"type":"frame","name":"Details","x":300,"y":0,"width":200,"height":100})),
            (
                button,
                serde_json::json!({"id":button,"type":"rect","name":"Go","parentId":first,"x":10,"y":10,"width":80,"height":30,
                    "interactions":[{"eventType":"click","actionType":"navigate","destination":second,
                        "animation":{"animationType":"slide","direction":"right","way":"in","duration":400,"easing":"ease-out"}}]}),
            ),
        ]
    }

    #[test]
    fn penpot_navigation_becomes_a_working_figcraft_interaction() {
        let (doc, warnings) = import(&archive_with_shapes(&prototype_shapes())).unwrap();
        assert!(doc.validate().is_empty(), "{warnings:?}");
        let button = doc.nodes().find(|n| n.name == "Go").unwrap();
        assert_eq!(button.interactions.len(), 1);
        let interaction = &button.interactions[0];
        assert_eq!(interaction.trigger, figcraft_doc::proto::Trigger::OnClick);
        assert_eq!(interaction.transition, figcraft_doc::proto::Transition::SlideIn { direction: figcraft_doc::proto::Direction::Right });
        assert_eq!(interaction.duration_ms, 400);
        assert_eq!(interaction.easing, figcraft_doc::proto::Easing::EaseOut);
        let Action::Navigate { dest, .. } = &interaction.actions[0] else { panic!("expected a navigation") };
        let details = doc.nodes().find(|n| n.name == "Details").unwrap();
        assert_eq!(*dest, details.id, "the destination is the frame the link names");
    }

    #[test]
    fn penpot_overlay_links_become_overlay_actions() {
        let button = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let overlay = "ffffffff-ffff-ffff-ffff-ffffffffffff";
        let shapes = [
            (
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
                serde_json::json!({"id":"cccccccc-cccc-cccc-cccc-cccccccccccc","type":"frame","name":"Home","width":200,"height":100}),
            ),
            (overlay, serde_json::json!({"id":overlay,"type":"frame","name":"Menu","x":300,"width":120,"height":60})),
            (
                button,
                serde_json::json!({"id":button,"type":"rect","name":"Menu button","parentId":"cccccccc-cccc-cccc-cccc-cccccccccccc",
                    "interactions":[{"eventType":"click","actionType":"open-overlay","destination":overlay,
                        "overlayPosType":"bottom-center","closeClickOutside":true,"backgroundOverlay":true}]}),
            ),
        ];
        let (doc, warnings) = import(&archive_with_shapes(&shapes)).unwrap();
        assert!(doc.validate().is_empty(), "{warnings:?}");
        let button = doc.nodes().find(|n| n.name == "Menu button").unwrap();
        let Action::OpenOverlay { dest, position, close_on_outside, background, .. } = &button.interactions[0].actions[0] else {
            panic!("expected an overlay action");
        };
        let menu = doc.nodes().find(|n| n.name == "Menu").unwrap();
        assert_eq!(*dest, menu.id);
        assert_eq!(*position, figcraft_doc::proto::OverlayPosition::BottomCenter);
        assert!(*close_on_outside);
        assert!(background.is_some(), "Penpot's overlay background comes across");
    }

    #[test]
    fn penpot_toggle_overlay_is_reported_and_preserved_without_becoming_open_overlay() {
        let button = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let overlay = "ffffffff-ffff-ffff-ffff-ffffffffffff";
        let shapes = [
            (
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
                serde_json::json!({"id":"cccccccc-cccc-cccc-cccc-cccccccccccc","type":"frame","name":"Home","width":200,"height":100}),
            ),
            (overlay, serde_json::json!({"id":overlay,"type":"frame","name":"Menu","x":300,"width":120,"height":60})),
            (
                button,
                serde_json::json!({"id":button,"type":"rect","name":"Menu button","parentId":"cccccccc-cccc-cccc-cccc-cccccccccccc",
                    "interactions":[{"eventType":"click","actionType":"toggle-overlay","destination":overlay,"overlayPosType":"center"}]}),
            ),
        ];
        let (doc, warnings) = import(&archive_with_shapes(&shapes)).unwrap();
        assert!(doc.validate().is_empty(), "{warnings:?}");
        let button = doc.nodes().find(|n| n.name == "Menu button").unwrap();
        assert!(button.interactions.is_empty());
        assert!(warnings.iter().any(|w| w.contains("toggle-overlay") && w.contains("skipped")));
        let (back, warnings) = import(&export(&doc).unwrap().0).unwrap();
        assert!(warnings.iter().any(|w| w.contains("toggle-overlay")));
        let back = back.nodes().find(|n| n.name == "Menu button").unwrap();
        assert_eq!(back.plugin_data.get("penpot.source.interactions"), button.plugin_data.get("penpot.source.interactions"));
    }

    #[test]
    fn penpot_links_to_nothing_are_skipped_with_a_note() {
        let button = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let shapes = [(
            button,
            serde_json::json!({"id":button,"type":"rect","name":"Dead link","width":10,"height":10,
                "interactions":[{"eventType":"click","actionType":"navigate","destination":"99999999-9999-9999-9999-999999999999"}]}),
        )];
        let (doc, warnings) = import(&archive_with_shapes(&shapes)).unwrap();
        assert!(doc.validate().is_empty(), "{warnings:?}");
        let button = doc.nodes().find(|n| n.name == "Dead link").unwrap();
        assert!(button.interactions.is_empty(), "a link to nowhere is not a working link");
        assert!(warnings.iter().any(|w| w.contains("named no layer")), "{warnings:?}");
    }

    #[test]
    fn penpot_page_flows_become_figcraft_flows() {
        let first = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        let shapes = prototype_shapes();
        let page = serde_json::json!({"id":"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb","name":"Canvas","index":0,
            "flows":{"11111111-1111-1111-1111-111111111111":{"id":"11111111-1111-1111-1111-111111111111","name":"Checkout","startingFrame":first}}});
        let (doc, warnings) = import(&archive_with_page(&shapes, &[], page)).unwrap();
        assert!(doc.validate().is_empty(), "{warnings:?}");
        let page_id = doc.first_page().unwrap();
        let NodeKind::Page { flows, .. } = &doc.node(page_id).unwrap().kind else { panic!("expected a page") };
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].name, "Checkout");
        let home = doc.nodes().find(|n| n.name == "Home").unwrap();
        assert_eq!(flows[0].start, home.id);
    }

    #[test]
    fn penpot_links_survive_a_save_and_reopen() {
        let (doc, warnings) = import(&archive_with_shapes(&prototype_shapes())).unwrap();
        assert!(doc.validate().is_empty(), "{warnings:?}");
        let saved = crate::save_bytes(&doc, None).unwrap();
        let (opened, issues) = crate::load_bytes(&saved).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        let button = opened.nodes().find(|n| n.name == "Go").unwrap();
        assert_eq!(button.interactions.len(), 1, "the prototype link survives the round trip");
    }
}
