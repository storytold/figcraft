//! Decoded `.fig` message → REST API file JSON, which [`crate::rest`] then imports. The mapping
//! is `craft_fig::rest`; FigCraft gives it its own connector routing and text measure, so an
//! imported connector runs as one drawn here, and takes the networks it hands over.

use craft_fig::rest::{self, Geometry, RouteEnd, Routing, SavedRuns, Side};
use figcraft_doc::geom::{Point, measure};
use figcraft_doc::{BoardShape, ConnectorPath, VectorNetwork};
use serde_json::Value;

use super::kiwi::Kv;

/// FigCraft's board routing and text measure, for the mapping.
struct FigGeometry;

fn end(e: RouteEnd) -> figcraft_doc::RouteEnd {
    match e {
        RouteEnd::Point(p) => figcraft_doc::RouteEnd::Point(p),
        RouteEnd::Box(b, s) => figcraft_doc::RouteEnd::Box(b, side(s)),
    }
}

fn side(s: Side) -> figcraft_doc::Side {
    match s {
        Side::Auto => figcraft_doc::Side::Auto,
        Side::Top => figcraft_doc::Side::Top,
        Side::Right => figcraft_doc::Side::Right,
        Side::Bottom => figcraft_doc::Side::Bottom,
        Side::Left => figcraft_doc::Side::Left,
    }
}

fn side_back(s: figcraft_doc::Side) -> Side {
    match s {
        figcraft_doc::Side::Auto => Side::Auto,
        figcraft_doc::Side::Top => Side::Top,
        figcraft_doc::Side::Right => Side::Right,
        figcraft_doc::Side::Bottom => Side::Bottom,
        figcraft_doc::Side::Left => Side::Left,
    }
}

impl Geometry for FigGeometry {
    fn route(&self, from: RouteEnd, to: RouteEnd, routing: Routing, saved: Option<&SavedRuns>) -> Option<Vec<Point>> {
        let routing = match routing {
            Routing::Straight => figcraft_doc::Routing::Straight,
            Routing::Elbow => figcraft_doc::Routing::Elbow,
            Routing::Curved => figcraft_doc::Routing::Curved,
        };
        let path = saved.map(|s| ConnectorPath::Runs { across: s.across, at: s.at.clone() });
        figcraft_doc::route_along(end(from), end(to), routing, path.as_ref())
    }

    fn attach(&self, from: RouteEnd, to: RouteEnd) -> ((Point, Option<Side>), (Point, Option<Side>)) {
        let (a, b) = (end(from), end(to));
        let ((p0, p1), (s0, s1)) = (figcraft_doc::attach_points(a, b), figcraft_doc::attached_sides(a, b));
        ((p0, s0.map(side_back)), (p1, s1.map(side_back)))
    }

    fn text_path_measurable(&self, d: &str) -> bool {
        let path = figcraft_doc::geom::svg_path(d).unwrap_or_default();
        measure::polylines(&path).and_then(|lines| lines.first().map(|line| line.length() > 1e-9)).unwrap_or(false)
    }
}

/// The REST API file JSON for message `msg`, with the warnings about what was skipped and the
/// vector networks its layers name by `vectorNetworkRef` (for [`crate::rest::import_with_networks`]).
#[cfg(test)]
pub(super) fn to_rest(msg: &Kv, name: &str, max_nodes: usize) -> (Value, Vec<String>, Vec<Option<VectorNetwork>>) {
    to_rest_for(msg, name, max_nodes, false)
}

/// Map a FigJam whiteboard's stickies as native board items when `board` is true.
pub(super) fn to_rest_for(msg: &Kv, name: &str, max_nodes: usize, board: bool) -> (Value, Vec<String>, Vec<Option<VectorNetwork>>) {
    let options = rest::Options { max_nodes, geometry: &FigGeometry, outlined_stroke: Some(super::export::STASH), board };
    let mut mapped = rest::to_rest(msg, name, &options);
    // Each network passes the importer's own checks, as the JSON's `vectorNetwork` does.
    let networks: Vec<Option<VectorNetwork>> =
        mapped.networks.into_iter().map(|n| serde_json::to_value(n?).ok().and_then(|v| crate::rest::network(&v))).collect();
    if board {
        native_board_shapes(msg, &mut mapped.json, &mut mapped.warnings);
    }
    (mapped.json, mapped.warnings, networks)
}

/// FigJam shapes whose generated records are complete are native board items in FigCraft. The
/// shared mapper owns the neutral generated layers; this adapter supplies only FigCraft's board
/// shape model, outline, and layout semantics.
fn native_board_shapes(msg: &Kv, json: &mut Value, warnings: &mut Vec<String>) {
    let Some(changes) = msg.get("nodeChanges") else { return };
    let mut shapes = std::collections::HashMap::new();
    for node in changes.list() {
        if node.get("type").and_then(Kv::str) != Some("SHAPE_WITH_TEXT") {
            continue;
        }
        let Some(guid) = node.get("guid") else { continue };
        let Some(origin) = guid
            .get("sessionID")
            .and_then(Kv::f64)
            .zip(guid.get("localID").and_then(Kv::f64))
            .map(|(session, local)| format!("{}:{}", session as u64, local as u64))
        else {
            continue;
        };
        let kind = node.get("shapeWithTextType").and_then(Kv::str).unwrap_or("SQUARE");
        let Some(shape) = board_shape(kind) else {
            warnings.push(format!("shape with text kind {} has no FigCraft outline; kept as a frame", crate::rest::props::clip_str(kind)));
            continue;
        };
        // An outer text fallback means the generated body was absent or invalid. Keep the shared
        // mapper's editable generated layers in that case, matching the older FigJam adapter.
        if node.get("textData").and_then(|t| t.get("characters")).and_then(Kv::str).is_some_and(|s| !s.trim().is_empty()) {
            continue;
        }
        shapes.insert(origin, (shape, generated_empty_body(node)));
    }
    apply_native_board_shapes(json, &shapes);
}

/// Traverse only the mapped REST tree (already bounded by `max_nodes`) and adapt each source
/// origin at most once. The origin index avoids rescanning the tree for every source record.
fn apply_native_board_shapes(json: &mut Value, shapes: &std::collections::HashMap<String, (BoardShape, bool)>) {
    fn visit(json: &mut Value, shapes: &std::collections::HashMap<String, (BoardShape, bool)>, visited: &mut std::collections::HashSet<String>) {
        if let Some(origin) = json.get(craft_fig::rest::FIG_SOURCE_FIELD).and_then(|s| s.get("origin")).and_then(Value::as_str).map(str::to_owned)
            && visited.insert(origin.clone())
            && let Some((shape, empty_body)) = shapes.get(&origin).copied()
        {
            apply_native_board_shape(json, &origin, shape, empty_body);
        }
        match json {
            Value::Object(fields) => fields.values_mut().for_each(|child| visit(child, shapes, visited)),
            Value::Array(items) => items.iter_mut().for_each(|child| visit(child, shapes, visited)),
            _ => {}
        }
    }
    visit(json, shapes, &mut std::collections::HashSet::new());
}

fn apply_native_board_shape(rest_node: &mut Value, origin: &str, shape: BoardShape, empty_body: bool) {
    let Some(children) = rest_node.get("children").and_then(Value::as_array) else { return };
    let has_generated_outline = !children.is_empty() && children[0]["type"] == "VECTOR";
    let has_generated_label = children.len() == 2 && children[1]["type"] == "TEXT";
    if !has_generated_outline || !(has_generated_label || (children.len() == 1 && empty_body)) {
        return;
    }
    let (Some(width), Some(height)) = (
        rest_node.get("size").and_then(|s| s.get("x")).and_then(Value::as_f64),
        rest_node.get("size").and_then(|s| s.get("y")).and_then(Value::as_f64),
    ) else {
        return;
    };
    if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
        return;
    }
    let (width, height) = (width.min(crate::rest::props::MAX_SIZE), height.min(crate::rest::props::MAX_SIZE));
    let Some(network) = serde_json::to_value(VectorNetwork::from_path(&shape.path(width, height))).ok() else { return };

    rest_node["board"] = serde_json::json!({"type": "shape", "shape": shape.id()});
    rest_node["fills"] = serde_json::json!([]);
    rest_node["strokes"] = serde_json::json!([]);
    rest_node["clipsContent"] = serde_json::json!(false);
    rest_node["layoutMode"] = serde_json::json!("VERTICAL");
    rest_node["primaryAxisAlignItems"] = serde_json::json!("CENTER");
    rest_node["counterAxisAlignItems"] = serde_json::json!("CENTER");
    rest_node["primaryAxisSizingMode"] = serde_json::json!("FIXED");
    rest_node["counterAxisSizingMode"] = serde_json::json!("FIXED");
    let [top, right, bottom, left] = shape.text_insets();
    rest_node["paddingTop"] = serde_json::json!(top * height);
    rest_node["paddingRight"] = serde_json::json!(right * width);
    rest_node["paddingBottom"] = serde_json::json!(bottom * height);
    rest_node["paddingLeft"] = serde_json::json!(left * width);

    let Some(children) = rest_node.get_mut("children").and_then(Value::as_array_mut) else { return };
    if children.len() == 1 {
        children.push(serde_json::json!({
            "id": format!("{origin};generated1"),
            "type": "TEXT",
            "name": "",
            "characters": "",
            "size": {"x": width, "y": height},
            "relativeTransform": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            "style": {"textAutoResize": "HEIGHT", "textAlignHorizontal": "CENTER"},
            "layoutSizingHorizontal": "FILL"
        }));
    }
    let outline = &mut children[0];
    outline["name"] = serde_json::json!("Outline");
    outline["vectorNetwork"] = network;
    outline["size"] = serde_json::json!({"x": width, "y": height});
    outline["relativeTransform"] = serde_json::json!([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
    outline["layoutPositioning"] = serde_json::json!("ABSOLUTE");
    outline["constraints"] = serde_json::json!({"horizontal": "SCALE", "vertical": "SCALE"});
    outline.as_object_mut().map(|o| o.remove("cornerRadius"));

    let label = &mut children[1];
    label["layoutSizingHorizontal"] = serde_json::json!("FILL");
    if let Some(style) = label.get_mut("style").and_then(Value::as_object_mut) {
        style.insert("textAutoResize".into(), serde_json::json!("HEIGHT"));
        style.insert("textAlignHorizontal".into(), serde_json::json!("CENTER"));
    }
}

/// The shared snapshot mapper omits a generated text child whose characters are empty. FigCraft
/// still keeps that empty child on a native shape, matching its prior mapper behavior.
fn generated_empty_body(node: &Kv) -> bool {
    fn record<'a, 's>(node: &'a Kv<'s>, field: &str) -> Option<&'a Kv<'s>> {
        node.get(field)?.get("overrides")?.list().iter().find(|record| {
            let Some(guids) = record.get("guidPath").and_then(|path| path.get("guids")).map(Kv::list) else { return false };
            if guids.len() != 1 {
                return false;
            }
            let session = guids[0].get("sessionID").and_then(Kv::f64);
            let local = guids[0].get("localID").and_then(Kv::f64);
            session == Some(40_000_000.0) && local == Some(1.0)
        })
    }
    let bounds = node.get("derivedImmutableFrameData");
    let version = bounds.and_then(|v| v.get("version")).and_then(Kv::f64);
    if version != Some(1.0) {
        return false;
    }
    let Some(style) = record(node, "nodeGenerationData") else { return false };
    let Some(_) = record(node, "derivedImmutableFrameData") else { return false };
    style.get("textData").and_then(|v| v.get("characters")).and_then(Kv::str) == Some("")
}

/// FigJam's `shapeWithTextType` values that have a corresponding FigCraft board outline.
pub(super) fn board_shape(kind: &str) -> Option<BoardShape> {
    Some(match kind {
        "SQUARE" => BoardShape::Rectangle,
        "ROUNDED_RECTANGLE" => BoardShape::RoundedRectangle,
        "ELLIPSE" => BoardShape::Ellipse,
        "DIAMOND" => BoardShape::Diamond,
        "TRIANGLE_UP" => BoardShape::Triangle,
        "TRIANGLE_DOWN" => BoardShape::TriangleDown,
        "PARALLELOGRAM_RIGHT" => BoardShape::ParallelogramRight,
        "PARALLELOGRAM_LEFT" => BoardShape::ParallelogramLeft,
        "TRAPEZOID" => BoardShape::Trapezoid,
        "PENTAGON" => BoardShape::Pentagon,
        "HEXAGON" => BoardShape::Hexagon,
        "OCTAGON" => BoardShape::Octagon,
        "STAR" => BoardShape::Star,
        "PLUS" => BoardShape::Plus,
        "ARROW_LEFT" => BoardShape::ArrowLeft,
        "ARROW_RIGHT" => BoardShape::ArrowRight,
        "CHEVRON" => BoardShape::Chevron,
        "SPEECH_BUBBLE" => BoardShape::SpeechBubble,
        "ENG_DATABASE" => BoardShape::Database,
        "DOCUMENT_SINGLE" => BoardShape::Document,
        "PREDEFINED_PROCESS" => BoardShape::PredefinedProcess,
        "MANUAL_INPUT" => BoardShape::ManualInput,
        "INTERNAL_STORAGE" => BoardShape::InternalStorage,
        _ => return None,
    })
}

/// REST importer warnings reworded for a `.fig` import (`None`: drop it).
pub(super) fn adapt_warning(w: String) -> Option<String> {
    if w.contains("geometry=paths") {
        return None;
    }
    Some(w.replace("the variables JSON", "the file").replace("the file JSON", "the file"))
}
