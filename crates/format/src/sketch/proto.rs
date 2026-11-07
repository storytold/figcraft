//! Sketch prototype links, hotspots, export options and layout grids → REST.
//!
//! A layer's `flow` links it to an artboard (`destinationArtboardID`, or `back`) with an
//! `animationType` (0 none, 1–4 slide from the right, left, bottom, top); hotspots are layers that
//! only carry a link; `isFlowHome` marks an artboard where a prototype starts.

use serde_json::{Map as JsonMap, Value, json};

use super::tree::{Cx, bool_field, str_field};

/// Most export settings read from one layer.
const MAX_EXPORTS: usize = 64;

/// The click interaction of layer `l`'s `flow`, if it has one.
pub(crate) fn interactions(l: &Value, node: &mut JsonMap<String, Value>) {
    let Some(flow) = l.get("flow") else { return };
    let Some(dest) = str_field(flow, "destinationArtboardID").filter(|d| !d.is_empty()) else { return };
    let action = if dest == "back" {
        json!({"type": "BACK"})
    } else {
        let direction = match flow.get("animationType").and_then(Value::as_i64) {
            Some(1) => Some("LEFT"),
            Some(2) => Some("RIGHT"),
            Some(3) => Some("TOP"),
            Some(4) => Some("BOTTOM"),
            _ => None,
        };
        let transition =
            direction.map_or(Value::Null, |d| json!({"type": "SLIDE_IN", "direction": d, "duration": 0.3, "easing": {"type": "EASE_IN_AND_OUT"}}));
        json!({"type": "NODE", "destinationId": dest, "navigation": "NAVIGATE", "transition": transition})
    };
    node.insert("interactions".into(), json!([{"trigger": {"type": "ON_CLICK"}, "actions": [action]}]));
}

/// The artboards of a page marked as where a prototype starts, as REST `flowStartingPoints`.
pub(crate) fn starting_points(page: &Value) -> Vec<Value> {
    super::tree::layers(page)
        .iter()
        .filter(|a| bool_field(a, "isFlowHome") == Some(true))
        .filter_map(|a| Some(json!({"nodeId": str_field(a, "do_objectID")?, "name": str_field(a, "name").unwrap_or("Flow")})))
        .collect()
}

/// Layer `l`'s export options as REST `exportSettings`.
pub(crate) fn export_settings(cx: &mut Cx, l: &Value, node: &mut JsonMap<String, Value>) {
    let formats = l.get("exportOptions").and_then(|o| o.get("exportFormats")).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let mut out = Vec::new();
    for f in formats.iter().take(MAX_EXPORTS) {
        let format = match str_field(f, "fileFormat").unwrap_or("png").to_ascii_lowercase().as_str() {
            "png" => "PNG",
            "jpg" | "jpeg" => "JPG",
            "svg" => "SVG",
            "pdf" => "PDF",
            "webp" => "WEBP",
            _ => {
                cx.warn("export settings in formats FigCraft doesn't export (TIFF, EPS, HEIC) were skipped");
                continue;
            }
        };
        let num = |k: &str| f.get(k).and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0);
        let constraint = match (f.get("visibleScaleType").and_then(Value::as_i64), num("absoluteSize")) {
            (Some(1), Some(s)) => json!({"type": "WIDTH", "value": s.min(16_384.0)}),
            (Some(2), Some(s)) => json!({"type": "HEIGHT", "value": s.min(16_384.0)}),
            _ => json!({"type": "SCALE", "value": num("scale").unwrap_or(1.0).min(64.0)}),
        };
        let name: String = str_field(f, "name").unwrap_or("").chars().take(200).collect();
        if f.get("namingScheme").and_then(Value::as_i64) == Some(1) && !name.is_empty() {
            cx.warn("export names Sketch puts before the layer name come after it");
        }
        out.push(json!({"format": format, "suffix": name, "constraint": constraint}));
    }
    if !out.is_empty() {
        node.insert("exportSettings".into(), Value::Array(out));
    }
}

/// An artboard's layout grid (columns, rows) and square grid as REST `layoutGrids`, hidden: Sketch
/// shows them only while its layout (or grid) view is turned on.
pub(crate) fn layout_grids(l: &Value, h: f64, node: &mut JsonMap<String, Value>) {
    let mut out = Vec::new();
    if let Some(g) = l.get("layout").filter(|g| bool_field(g, "isEnabled") != Some(false)) {
        let num = |k: &str| g.get(k).and_then(Value::as_f64).filter(|v| v.is_finite() && *v >= 0.0);
        if bool_field(g, "drawVertical") != Some(false)
            && let (Some(count), Some(width)) = (num("numberOfColumns"), num("columnWidth"))
        {
            out.push(json!({"pattern": "COLUMNS", "alignment": "MIN", "count": count.min(1000.0), "sectionSize": width.min(1e5),
                "gutterSize": num("gutterWidth").unwrap_or(0.0).min(1e5), "offset": num("horizontalOffset").unwrap_or(0.0).min(1e5), "visible": false}));
        }
        if bool_field(g, "drawHorizontal") == Some(true) {
            let gutter = num("gutterHeight").unwrap_or(0.0).min(1e5);
            let section = (gutter * num("rowHeightMultiplication").unwrap_or(1.0)).min(1e5);
            if section > 0.0 {
                let count = (h / (section + gutter)).floor().clamp(1.0, 1000.0);
                out.push(json!({"pattern": "ROWS", "alignment": "MIN", "count": count, "sectionSize": section, "gutterSize": gutter, "offset": 0, "visible": false}));
            }
        }
    }
    if let Some(g) = l.get("grid").filter(|g| bool_field(g, "isEnabled") != Some(false))
        && let Some(size) = g.get("gridSize").and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0)
    {
        out.push(json!({"pattern": "GRID", "sectionSize": size.min(1e5), "visible": false}));
    }
    if !out.is_empty() {
        node.insert("layoutGrids".into(), Value::Array(out));
    }
}

/// Notes about what the layer has that FigCraft keeps only as laid out: Smart Layout,
/// and overrides a master locks.
pub(crate) fn notes(cx: &mut Cx, l: &Value) {
    // Stacks become auto layout; the older Smart Layout doesn't.
    if l.get("groupLayout").and_then(|g| str_field(g, "_class")) == Some("MSImmutableInferredGroupLayout") {
        cx.warn("Smart Layout was imported as laid out; it doesn't resize with its content");
    }
    let locked = l.get("overrideProperties").and_then(Value::as_array).is_some_and(|p| p.iter().any(|o| bool_field(o, "canOverride") == Some(false)));
    if locked {
        cx.warn("overrides Sketch symbols locked can be changed in FigCraft");
    }
}
