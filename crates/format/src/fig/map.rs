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
    snap_table_cells(msg, &mut mapped.json, &mut mapped.warnings);
    connector_label_pills(&mut mapped.json, max_nodes, &mut mapped.warnings);
    // Each network passes the importer's own checks, as the JSON's `vectorNetwork` does.
    let networks: Vec<Option<VectorNetwork>> =
        mapped.networks.into_iter().map(|n| serde_json::to_value(n?).ok().and_then(|v| crate::rest::network(&v))).collect();
    if board {
        native_board_shapes(msg, &mut mapped.json, &mut mapped.warnings);
        let native = native_board_connectors(msg, &mut mapped.json);
        uncount(&mut mapped.warnings, "connector layers import as vectors", native);
    }
    (mapped.json, mapped.warnings, networks)
}

/// Every layer of the mapped REST tree (already bounded by `max_nodes`), parents before their
/// children, walked with an explicit stack so a deep file can't exhaust the call stack.
fn each_tree_layer(json: &mut Value, f: &mut impl FnMut(&mut Value)) {
    let Some(document) = json.get_mut("document") else { return };
    let mut pending = vec![document];
    while let Some(layer) = pending.pop() {
        f(layer);
        if let Some(children) = layer.get_mut("children").and_then(Value::as_array_mut) {
            pending.extend(children.iter_mut().rev());
        }
    }
}

/// How many layers the mapped REST tree holds (the document and its pages included).
fn tree_layer_count(json: &mut Value) -> usize {
    let mut count = 0;
    each_tree_layer(json, &mut |_| count += 1);
    count
}

/// Note `what` once, or with its count (`what (×n)`) as the shared mapper counts its notes; none at 0.
fn warn(what: &'static str, n: usize, warnings: &mut Vec<String>) {
    match n {
        0 => {}
        1 => warnings.push(what.to_string()),
        n => warnings.push(format!("{what} (×{n})")),
    }
}

/// The shared mapper's warning for a label without its saved snapshot.
const BARE_LABEL_WARNING: &str = "connector labels without a saved snapshot are centred on the line, without their background";
/// Average advance of a glyph as a fraction of the font size: the pill is sized before the fonts
/// lay the label out, so it can only estimate the text's width.
const LABEL_ADVANCE: f64 = 0.6;

/// A connector with a label but no saved snapshot of its background comes from the shared mapper
/// as a frame holding its line (`;line`) and its label (`;label`), the label bare on the line.
/// Each such label gets a white pill behind it (centred on the label, padded by half the font size
/// across and a quarter of it down, fully rounded), as a labelled connector drawn here shows. At
/// the layer budget the remaining labels stay bare, under the shared mapper's warning.
fn connector_label_pills(json: &mut Value, max_nodes: usize, warnings: &mut Vec<String>) {
    if !warnings.iter().any(|w| w.starts_with(BARE_LABEL_WARNING)) {
        return;
    }
    let mut room = max_nodes.saturating_sub(tree_layer_count(json));
    let mut added = 0;
    each_tree_layer(json, &mut |layer| {
        let Some(id) = layer.get("id").and_then(Value::as_str).map(str::to_owned) else { return };
        let Some(children) = layer.get_mut("children").and_then(Value::as_array_mut) else { return };
        let is_fallback = children.len() == 2
            && children[0].get("id").and_then(Value::as_str) == Some(format!("{id};line").as_str())
            && children[1].get("id").and_then(Value::as_str) == Some(format!("{id};label").as_str());
        if !is_fallback {
            return;
        }
        let Some(pill) = label_pill(&id, &children[1]) else { return };
        if room == 0 {
            return;
        }
        children.insert(1, pill);
        room -= 1;
        added += 1;
    });
    uncount(warnings, BARE_LABEL_WARNING, added);
    warn("connector labels without a saved snapshot are centred on the line, on a stand-in background pill", added, warnings);
}

/// The background pill for fallback connector label `label` of connector `id`; `None` when the
/// label is blank or its box isn't a finite, positive, unrotated one.
fn label_pill(id: &str, label: &Value) -> Option<Value> {
    let positive = |v: Option<f64>| v.filter(|v| v.is_finite() && *v > 0.0);
    let characters = label.get("characters").and_then(Value::as_str)?;
    let longest = characters.lines().map(|line| line.trim().chars().count()).max().filter(|n| *n > 0)?;
    let font_size = positive(label.get("style").and_then(|s| s.get("fontSize")).and_then(Value::as_f64)).unwrap_or(16.0);
    let size = label.get("size")?;
    let (box_w, box_h) = (positive(size.get("x").and_then(Value::as_f64))?, positive(size.get("y").and_then(Value::as_f64))?);
    let rows = label.get("relativeTransform")?;
    let cell = |r: usize, c: usize| rows.get(r).and_then(|row| row.get(c)).and_then(Value::as_f64).filter(|v| v.is_finite());
    if (cell(0, 0)?, cell(0, 1)?, cell(1, 0)?, cell(1, 1)?) != (1.0, 0.0, 0.0, 1.0) {
        return None;
    }
    let (x, y) = (cell(0, 2)?, cell(1, 2)?);
    let (centre_x, centre_y) = (x + box_w / 2.0, y + box_h / 2.0);
    let text_w = (longest as f64 * font_size * LABEL_ADVANCE).min(box_w);
    let max = crate::rest::props::MAX_SIZE;
    let (w, h) = ((text_w + font_size).min(max), (box_h + font_size / 2.0).min(max));
    Some(serde_json::json!({
        "id": format!("{id};labelBackground"),
        "type": "RECTANGLE",
        "name": "Label background",
        "size": {"x": w, "y": h},
        "relativeTransform": [[1.0, 0.0, centre_x - w / 2.0], [0.0, 1.0, centre_y - h / 2.0]],
        "fills": [{"type": "SOLID", "color": {"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0}}],
        "strokes": [],
        "cornerRadius": h / 2.0
    }))
}

/// Cells further apart than this (in the table's units) start a new row or column.
const TRACK_SNAP: f64 = 2.0;
/// Largest table whose cells are snapped to a grid; bigger ones keep their saved bounds.
pub(super) const MAX_TABLE_CELLS: usize = 4096;
/// Most rows or columns a snapped table may have.
const MAX_TABLE_TRACKS: usize = 512;

/// The origins of the message's `TABLE` and `TABLE_CELL` records.
fn table_origins(msg: &Kv) -> (std::collections::HashSet<String>, std::collections::HashSet<String>) {
    let (mut tables, mut cells) = (std::collections::HashSet::new(), std::collections::HashSet::new());
    for node in msg.get("nodeChanges").map(Kv::list).unwrap_or_default() {
        let set = match node.get("type").and_then(Kv::str) {
            Some("TABLE") => &mut tables,
            Some("TABLE_CELL") => &mut cells,
            _ => continue,
        };
        if let Some(origin) = node.get("guid").and_then(origin) {
            set.insert(origin);
        }
    }
    (tables, cells)
}

/// One table cell as saved: its offset in the table and its size when the file gives a valid one.
struct SavedCell {
    child: usize,
    x: f64,
    y: f64,
    size: Option<(f64, f64)>,
}

/// A table comes from the shared mapper as a frame holding its cells as frames, each at the
/// bounds the file saved for it: a cell without a size is empty, and cells saved a pixel or two
/// apart overlap or leave gaps. The cells are put back on the table's grid: their offsets are
/// grouped into rows and columns ([`TRACK_SNAP`]), each track runs to the next one (the last to
/// the furthest cell edge, else the table's own edge), and each cell takes the tracks it starts
/// and ends on, so no two cells overlap. Fills, strokes and text stay as they were; a cell's text
/// keeps its inset. A table this can't make sense of (a rotated or non-finite cell, two cells in
/// one slot, more than [`MAX_TABLE_CELLS`] cells) keeps its saved bounds, with a warning.
fn snap_table_cells(msg: &Kv, json: &mut Value, warnings: &mut Vec<String>) {
    let (tables, cells) = table_origins(msg);
    if tables.is_empty() || cells.is_empty() {
        return;
    }
    let (mut snapped, mut kept) = (0usize, 0usize);
    each_tree_layer(json, &mut |table| {
        if !layer_origin(table).is_some_and(|o| tables.contains(o)) {
            return;
        }
        match snap_table(table, &cells) {
            Some(n) => snapped += n,
            None => kept += 1,
        }
    });
    warn("table cells were snapped to their table's rows and columns", snapped, warnings);
    warn("tables whose rows and columns couldn't be told kept their cells' saved bounds", kept, warnings);
}

/// Snap `table`'s cells to its grid. Returns how many cells moved or changed size (`Some(0)` for
/// a table without cells or already on its grid); `None` when the cells were left as saved.
fn snap_table(table: &mut Value, cells: &std::collections::HashSet<String>) -> Option<usize> {
    let positive = |v: Option<f64>| v.filter(|v| v.is_finite() && *v > 0.0 && *v <= crate::rest::props::MAX_SIZE);
    let extent = |axis: &str| positive(table.get("size").and_then(|s| s.get(axis)).and_then(Value::as_f64));
    let (table_w, table_h) = (extent("x"), extent("y"));
    let children = table.get_mut("children").and_then(Value::as_array_mut)?;
    let mut saved = Vec::new();
    for (child, layer) in children.iter().enumerate() {
        if !layer_origin(layer).is_some_and(|o| cells.contains(o)) {
            continue;
        }
        if saved.len() == MAX_TABLE_CELLS {
            return None;
        }
        let (x, y) = translation(layer)?;
        let size = layer.get("size");
        let axis = |a: &str| positive(size.and_then(|s| s.get(a)).and_then(Value::as_f64));
        saved.push(SavedCell { child, x, y, size: axis("x").zip(axis("y")) });
    }
    if saved.is_empty() {
        return Some(0);
    }
    let columns = tracks(saved.iter().map(|c| (c.x, c.size.map(|s| c.x + s.0))), table_w)?;
    let rows = tracks(saved.iter().map(|c| (c.y, c.size.map(|s| c.y + s.1))), table_h)?;
    let mut taken = vec![false; (columns.len() - 1).checked_mul(rows.len() - 1).filter(|n| *n <= MAX_TABLE_CELLS * 16)?];
    let mut placed = Vec::with_capacity(saved.len());
    for cell in &saved {
        let (c0, c1) = span(&columns, cell.x, cell.size.map(|s| cell.x + s.0));
        let (r0, r1) = span(&rows, cell.y, cell.size.map(|s| cell.y + s.1));
        // A cell put in a track it doesn't start near (the grouping and the nearest track disagree)
        // means the grid wasn't told right: keep the table as saved.
        if (columns[c0] - cell.x).abs() > TRACK_SNAP || (rows[r0] - cell.y).abs() > TRACK_SNAP {
            return None;
        }
        for r in r0..r1 {
            for c in c0..c1 {
                let slot = taken.get_mut(r * (columns.len() - 1) + c)?;
                if *slot {
                    return None;
                }
                *slot = true;
            }
        }
        placed.push((cell.child, columns[c0], rows[r0], columns[c1] - columns[c0], rows[r1] - rows[r0], cell));
    }
    let mut moved = 0;
    for (child, x, y, w, h, cell) in placed {
        if (cell.x, cell.y, cell.size) == (x, y, Some((w, h))) {
            continue;
        }
        moved += 1;
        let layer = &mut children[child];
        layer["relativeTransform"] = serde_json::json!([[1.0, 0.0, x], [0.0, 1.0, y]]);
        layer["size"] = serde_json::json!({"x": w, "y": h});
        let id = layer.get("id").and_then(Value::as_str).map(|id| format!("{id};text"));
        let Some(text) = layer
            .get_mut("children")
            .and_then(Value::as_array_mut)
            .and_then(|kids| kids.iter_mut().find(|kid| kid.get("id").and_then(Value::as_str) == id.as_deref()))
        else {
            continue;
        };
        if let Some((inset_x, inset_y)) = translation(text) {
            text["size"] = serde_json::json!({"x": (w - 2.0 * inset_x).max(0.0), "y": (h - 2.0 * inset_y).max(0.0)});
        }
    }
    Some(moved)
}

/// The offset of a layer whose `relativeTransform` is a finite, unrotated, unscaled translation.
fn translation(layer: &Value) -> Option<(f64, f64)> {
    let rows = layer.get("relativeTransform")?;
    let cell = |r: usize, c: usize| rows.get(r).and_then(|row| row.get(c)).and_then(Value::as_f64).filter(|v| v.is_finite());
    let linear = (cell(0, 0)?, cell(0, 1)?, cell(1, 0)?, cell(1, 1)?);
    let max = crate::rest::props::MAX_SIZE;
    let (x, y) = (cell(0, 2)?, cell(1, 2)?);
    (linear == (1.0, 0.0, 0.0, 1.0) && x.abs() <= max && y.abs() <= max).then_some((x, y))
}

/// One axis of a table's grid from its cells' `(start, end)` offsets: the track boundaries in
/// order, the last one closing the last track. `None` when the tracks can't be told (too many, or
/// the last one has no extent).
fn tracks(cells: impl Iterator<Item = (f64, Option<f64>)>, extent: Option<f64>) -> Option<Vec<f64>> {
    let (mut starts, mut far) = (Vec::new(), None::<f64>);
    for (start, end) in cells {
        starts.push(start);
        if let Some(end) = end {
            far = Some(far.map_or(end, |f| f.max(end)));
        }
    }
    starts.sort_by(f64::total_cmp);
    let mut boundaries: Vec<f64> = Vec::new();
    let mut group: Vec<f64> = Vec::new();
    for start in starts {
        if group.last().is_some_and(|last| start - last > TRACK_SNAP) {
            boundaries.push(group.iter().sum::<f64>() / group.len() as f64);
            group.clear();
        }
        group.push(start);
    }
    if !group.is_empty() {
        boundaries.push(group.iter().sum::<f64>() / group.len() as f64);
    }
    if boundaries.len() > MAX_TABLE_TRACKS {
        return None;
    }
    let last = *boundaries.last()?;
    let previous = boundaries.len().checked_sub(2).map(|i| last - boundaries[i]);
    let end = [far, extent, previous.map(|w| last + w)].into_iter().flatten().find(|end| *end > last + TRACK_SNAP)?;
    boundaries.push(end);
    Some(boundaries)
}

/// The tracks `[first, last)` of `boundaries` a cell from `start` to `end` covers: the track it
/// starts in, through the boundary nearest its end (one track when it has no end).
fn span(boundaries: &[f64], start: f64, end: Option<f64>) -> (usize, usize) {
    let nearest = |v: f64| (0..boundaries.len()).min_by(|a, b| (boundaries[*a] - v).abs().total_cmp(&(boundaries[*b] - v).abs())).unwrap_or(0);
    let first = nearest(start).min(boundaries.len().saturating_sub(2));
    let last = end.map_or(first + 1, nearest).clamp(first + 1, boundaries.len() - 1);
    (first, last)
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

/// A record's `guid` as the `session:local` origin the mapped layers carry.
fn origin(guid: &Kv) -> Option<String> {
    let (session, local) = guid.get("sessionID").and_then(Kv::f64).zip(guid.get("localID").and_then(Kv::f64))?;
    Some(format!("{}:{}", session as u64, local as u64))
}

/// Every mapped layer of `json` (bounded by `max_nodes` already), depth first.
fn each_layer(json: &mut Value, f: &mut impl FnMut(&mut Value)) {
    if json.get(craft_fig::rest::FIG_SOURCE_FIELD).is_some() {
        f(json);
    }
    match json {
        Value::Object(fields) => fields.values_mut().for_each(|child| each_layer(child, f)),
        Value::Array(items) => items.iter_mut().for_each(|child| each_layer(child, f)),
        _ => {}
    }
}

/// A mapped layer's source origin.
fn layer_origin(layer: &Value) -> Option<&str> {
    layer.get(craft_fig::rest::FIG_SOURCE_FIELD)?.get("origin")?.as_str()
}

/// A whiteboard file's connectors are native board connectors: the line the shared mapper drew
/// stays, and the connector is attached to the layers it joins, so FigCraft routes it again when
/// they move. Only a connector drawn as one line on the page itself is adapted: a free end's
/// point is in its parent's space, the page's only there, and one with a label is a frame holding
/// its line and label, which stays as it is.
/// Returns how many it attached.
fn native_board_connectors(msg: &Kv, json: &mut Value) -> usize {
    let Some(changes) = msg.get("nodeChanges") else { return 0 };
    let ty = |n: &Kv| n.get("type").and_then(Kv::str).map(str::to_owned);
    let pages: std::collections::HashSet<String> =
        changes.list().iter().filter(|n| ty(n).as_deref() == Some("CANVAS")).filter_map(|n| origin(n.get("guid")?)).collect();
    let mut connectors = std::collections::HashMap::new();
    for node in changes.list() {
        if ty(node).as_deref() != Some("CONNECTOR")
            || node.get("parentIndex").and_then(|p| p.get("guid")).and_then(origin).is_none_or(|p| !pages.contains(&p))
        {
            continue;
        }
        let Some(id) = node.get("guid").and_then(origin) else { continue };
        let end = |key: &str| -> Option<Value> {
            let e = node.get(key)?;
            let Some(target) = e.get("endpointNodeID").and_then(origin) else {
                let p = e.get("position")?;
                return Some(serde_json::json!({"x": p.get("x").and_then(Kv::f64)?, "y": p.get("y").and_then(Kv::f64)?}));
            };
            let magnet = e.get("magnet").and_then(Kv::str);
            let side = match magnet {
                Some("TOP") => "top",
                Some("RIGHT") => "right",
                Some("BOTTOM") => "bottom",
                Some("LEFT") => "left",
                _ => "auto",
            };
            let mut out = serde_json::json!({"node": target, "side": side});
            // Without a magnet the end sits where it was put on the layer.
            if matches!(magnet, None | Some("NONE"))
                && let Some(r) = e.get("relativePosition")
                && let Some((x, y)) = r.get("x").and_then(Kv::f64).zip(r.get("y").and_then(Kv::f64))
            {
                out["at"] = serde_json::json!([x, y]);
            }
            Some(out)
        };
        let routing = match node.get("connectorLineStyle").and_then(Kv::str) {
            Some("ELBOWED") => "elbow",
            Some("CURVED") => "curved",
            _ => "straight",
        };
        if let Some((from, to)) = end("connectorStart").zip(end("connectorEnd")) {
            connectors.insert(id, serde_json::json!({"type": "connector", "from": from, "to": to, "routing": routing}));
        }
    }
    if connectors.is_empty() {
        return 0;
    }
    // The mapped layers' REST ids by origin: a connector's ends name the layers it joins by them.
    let mut ids = std::collections::HashMap::new();
    let mut attached = 0;
    each_layer(json, &mut |layer| {
        if let (Some(origin), Some(id)) = (layer_origin(layer), layer.get("id").and_then(Value::as_str)) {
            ids.entry(origin.to_owned()).or_insert_with(|| id.to_owned());
        }
    });
    each_layer(json, &mut |layer| {
        if layer["type"] != "VECTOR" {
            return;
        }
        let Some(mut board) = layer_origin(layer).and_then(|o| connectors.get(o)).cloned() else { return };
        for key in ["from", "to"] {
            if let Some(node) = board[key].get("node").and_then(Value::as_str) {
                match ids.get(node) {
                    Some(id) => board[key]["node"] = Value::String(id.clone()),
                    // A layer left out (or not a layer here) leaves the connector a plain line.
                    None => return,
                }
            }
        }
        layer["board"] = board;
        attached += 1;
    });
    attached
}

/// Take `n` off the count of warning `what` (`what (×count)`, or `what` once), dropping it at none.
fn uncount(warnings: &mut Vec<String>, what: &str, n: usize) {
    let Some(i) = warnings.iter().position(|w| w.starts_with(what)) else { return };
    let count = warnings[i].strip_prefix(what).and_then(|r| r.trim().strip_prefix("(×")?.strip_suffix(')')?.parse::<usize>().ok()).unwrap_or(1);
    match count.saturating_sub(n) {
        0 => _ = warnings.remove(i),
        1 => warnings[i] = what.to_string(),
        left => warnings[i] = format!("{what} (×{left})"),
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
