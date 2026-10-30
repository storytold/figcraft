//! Document inspection as JSON, so agents can verify their work without screenshots.

use figcraft_doc::{Document, Node, NodeId, NodeKind, StrokeJoin, TextData, TextDecoration};
use serde_json::{Value, json};

use crate::Session;

/// Top-level node keys accepted by `fields`, including the structure kept in every tree.
pub const NODE_KEYS: &[&str] = &[
    "absolute",
    "absoluteBounds",
    "annotations",
    "arc",
    "background",
    "blend",
    "board",
    "booleanOperation",
    "boundVariables",
    "brush",
    "characters",
    "childCount",
    "children",
    "clip",
    "codeLinks",
    "componentProperties",
    "componentPropertyDefinitions",
    "constraints",
    "cornerRadius",
    "cornerSmoothing",
    "dashes",
    "description",
    "devChanged",
    "devNote",
    "devResources",
    "devStatus",
    "effects",
    "exports",
    "fills",
    "flows",
    "fontFamily",
    "fontSize",
    "fontStyle",
    "fontVariations",
    "gridCell",
    "grids",
    "hangingList",
    "hangingPunctuation",
    "height",
    "id",
    "innerRadius",
    "interactions",
    "isMask",
    "layout",
    "leadingTrim",
    "letterSpacing",
    "lineHeight",
    "lineIndentations",
    "lineTypes",
    "listSpacing",
    "listStartOffsets",
    "listStyle",
    "locked",
    "mainComponent",
    "mainComponentName",
    "maskType",
    "maxHeight",
    "maxLines",
    "maxWidth",
    "measurements",
    "minHeight",
    "minWidth",
    "miterAngle",
    "name",
    "opacity",
    "operandOperation",
    "overrides",
    "pluginData",
    "pointCount",
    "rotation",
    "sectionContentsHidden",
    "segments",
    "sizing",
    "slot",
    "strokeAlign",
    "strokeJoin",
    "strokeSides",
    "strokeWeight",
    "strokes",
    "styledRuns",
    "styles",
    "textAlign",
    "textAutoResize",
    "textDecoration",
    "textDecorationColor",
    "textDecorationOffset",
    "textDecorationSkipInk",
    "textDecorationStyle",
    "textDecorationThickness",
    "textPath",
    "textStyleRanges",
    "type",
    "vertices",
    "visible",
    "width",
    "widthProfile",
    "x",
    "y",
];

/// Which layer properties to build. Missing options keep the full, sparse inspection output.
#[derive(Clone, Debug, Default)]
pub struct InspectOptions {
    fields: Option<std::collections::BTreeSet<&'static str>>,
    outline: bool,
}

impl InspectOptions {
    /// Parse the same options for engine commands, control methods and MCP tools.
    pub fn parse(p: &Value) -> Result<Self, String> {
        let outline = match p.get("outline") {
            None => false,
            Some(Value::Bool(v)) => *v,
            Some(_) => return Err("`outline` must be a boolean".into()),
        };
        if outline && p.get("fields").is_some() {
            return Err("`outline: true` cannot be combined with `fields`".into());
        }
        let fields = match p.get("fields") {
            None if outline => Some(["id", "name", "type", "characters", "mainComponentName", "visible"].into_iter().collect()),
            None => None,
            Some(Value::Array(values)) => {
                let mut fields = std::collections::BTreeSet::new();
                for value in values {
                    let key = value.as_str().ok_or("`fields` must be an array of strings")?;
                    let key = NODE_KEYS
                        .iter()
                        .copied()
                        .find(|k| *k == key)
                        .ok_or_else(|| format!("unknown field `{key}`; valid keys: {}", NODE_KEYS.join(", ")))?;
                    fields.insert(key);
                }
                Some(fields)
            }
            Some(_) => return Err("`fields` must be an array of strings".into()),
        };
        Ok(Self { fields, outline })
    }

    fn includes(&self, key: &str) -> bool {
        self.fields.as_ref().is_none_or(|fields| fields.contains(key))
    }

    fn explicit(&self, key: &str) -> bool {
        self.fields.as_ref().is_some_and(|fields| fields.contains(key))
    }
}

/// A node and (to `depth`) its children, with the properties a designer would check.
pub fn node_json(doc: &Document, id: NodeId, depth: usize) -> Value {
    node_json_with_options(doc, id, depth, &InspectOptions::default())
}

/// A node's selected properties, applied at every level of its subtree.
pub fn node_json_with_options(doc: &Document, id: NodeId, depth: usize, options: &InspectOptions) -> Value {
    let Some(n) = doc.node(id) else { return Value::Null };
    let mut v = json!({"id": n.id});
    let Some(o) = v.as_object_mut() else { return v };
    insert_prop(o, options, "name", || json!(n.name));
    insert_prop(o, options, "type", || json!(n.kind.type_name()));
    if options.explicit("visible") {
        insert_prop(o, options, "visible", || json!(n.visible));
    }
    if !n.is_page() {
        base_props(doc, n, id, o, options);
        paint_props(n, o, options);
        layout_props(doc, n, o, options);
        dev_props(doc, n, o, options);
    }
    // Layer data written with `node.set {props: {pluginData}}` (pages too).
    if !n.plugin_data.is_empty() {
        insert_prop(o, options, "pluginData", || json!(n.plugin_data));
    }
    match &n.kind {
        NodeKind::Text(t) => text_props(t, o, options),
        _ => kind_props(doc, n, o, options),
    }
    if !n.children.is_empty() {
        if depth > 0 {
            o.insert("children".into(), Value::Array(n.children.iter().map(|c| node_json_with_options(doc, *c, depth - 1, options)).collect()));
        } else {
            o.insert("childCount".into(), json!(n.children.len()));
        }
    }
    v
}

/// Build a property only when it was requested (or no field list was given).
fn insert_prop(o: &mut serde_json::Map<String, Value>, options: &InspectOptions, key: &str, value: impl FnOnce() -> Value) {
    if options.includes(key) {
        o.insert(key.into(), value());
    }
}

fn base_props(doc: &Document, n: &Node, id: NodeId, o: &mut serde_json::Map<String, Value>, options: &InspectOptions) {
    if options.includes("x") || options.includes("y") {
        let c = doc.frame_relative(id).as_coeffs();
        insert_prop(o, options, "x", || json!(round(c[4])));
        insert_prop(o, options, "y", || json!(round(c[5])));
    }
    insert_prop(o, options, "width", || json!(round(n.width)));
    insert_prop(o, options, "height", || json!(round(n.height)));
    if options.includes("rotation") {
        let rot = figcraft_geom::rotation_degrees(n.transform);
        if rot.abs() > 1e-6 {
            insert_prop(o, options, "rotation", || json!(round(rot)));
        }
    }
    if let Some(b) = options.includes("absoluteBounds").then(|| doc.world_bounds(id)).flatten() {
        insert_prop(
            o,
            options,
            "absoluteBounds",
            || json!({"x": round(b.x0), "y": round(b.y0), "width": round(b.width()), "height": round(b.height())}),
        );
    }
    if !n.visible {
        insert_prop(o, options, "visible", || json!(false));
    }
    if n.locked {
        insert_prop(o, options, "locked", || json!(true));
    }
    if n.opacity < 1.0 {
        insert_prop(o, options, "opacity", || json!(n.opacity));
    }
    if !matches!(n.blend, figcraft_color::BlendMode::Normal | figcraft_color::BlendMode::PassThrough) {
        insert_prop(o, options, "blend", || json!(n.blend));
    }
}

fn paint_props(n: &Node, o: &mut serde_json::Map<String, Value>, options: &InspectOptions) {
    if !n.fills.is_empty() {
        insert_prop(o, options, "fills", || paints(&n.fills));
    }
    if !n.strokes.is_empty() {
        insert_prop(o, options, "strokes", || paints(&n.strokes));
        insert_prop(o, options, "strokeWeight", || json!(n.stroke.weight));
        insert_prop(o, options, "strokeAlign", || json!(n.stroke.align));
        if n.stroke.sides.is_some() {
            insert_prop(o, options, "strokeSides", || json!(n.stroke.sides));
        }
        if !n.stroke.dashes.is_empty() {
            insert_prop(o, options, "dashes", || json!(n.stroke.dashes));
        }
        if n.stroke.join != StrokeJoin::Miter {
            insert_prop(o, options, "strokeJoin", || json!(n.stroke.join));
        } else if n.stroke.miter_limit != 4.0 {
            insert_prop(o, options, "miterAngle", || json!((n.stroke.miter_angle() * 100.0).round() / 100.0));
        }
        if let Some(p) = n.stroke.profile() {
            insert_prop(o, options, "widthProfile", || {
                let points = crate::cmd::props::width_points_json(Some(p));
                json!({"preset": p.preset().map(|x| x.id()), "points": points})
            });
        }
        if let Some(b) = &n.stroke.brush {
            insert_prop(o, options, "brush", || camel(serde_json::to_value(b).unwrap_or(Value::Null)));
        }
    }
    if !n.effects.is_empty() {
        insert_prop(o, options, "effects", || camel(serde_json::to_value(&n.effects).unwrap_or(Value::Null)));
    }
    if n.corners.radii.iter().any(|r| *r > 0.0) {
        insert_prop(o, options, "cornerRadius", || match n.corners.uniform() {
            Some(r) => json!(r),
            None => json!(n.corners.radii),
        });
        if n.corners.smoothing > 0.0 {
            insert_prop(o, options, "cornerSmoothing", || json!(n.corners.smoothing));
        }
    }
}

fn layout_props(doc: &Document, n: &Node, o: &mut serde_json::Map<String, Value>, options: &InspectOptions) {
    if let Some(l) = &n.layout {
        insert_prop(o, options, "layout", || camel(serde_json::to_value(l).unwrap_or(Value::Null)));
    }
    let parent_al = n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some());
    if n.layout.is_some() || parent_al || matches!(n.kind, NodeKind::Text(_)) {
        insert_prop(o, options, "sizing", || json!({"horizontal": n.child_layout.sizing_h, "vertical": n.child_layout.sizing_v}));
    }
    let cl = &n.child_layout;
    if cl.absolute {
        insert_prop(o, options, "absolute", || json!(true));
    }
    for (k, v) in [("minWidth", cl.min_w), ("maxWidth", cl.max_w), ("minHeight", cl.min_h), ("maxHeight", cl.max_h)] {
        if let Some(v) = v {
            insert_prop(o, options, k, || json!(v));
        }
    }
    if let Some(cell) = (parent_al && options.includes("gridCell"))
        .then(|| n.parent.and_then(|p| figcraft_layout::grid_geometry(doc, p)))
        .flatten()
        .and_then(|g| g.cells.into_iter().find(|c| c.id == n.id))
    {
        insert_prop(o, options, "gridCell", || {
            json!({"column": cell.column, "row": cell.row, "columnSpan": cell.column_span, "rowSpan": cell.row_span,
                       "anchored": cl.grid_column.is_some() || cl.grid_row.is_some(), "alignH": cl.grid_align_h, "alignV": cl.grid_align_v})
        });
    }
    if (!parent_al || cl.absolute) && n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.kind.is_frame_like()) {
        insert_prop(o, options, "constraints", || serde_json::to_value(n.constraints).unwrap_or(Value::Null));
    }
    if n.kind.is_frame_like() {
        insert_prop(o, options, "clip", || json!(n.clip));
    }
}

fn dev_props(doc: &Document, n: &Node, o: &mut serde_json::Map<String, Value>, options: &InspectOptions) {
    if !n.dev_status.is_none() {
        insert_prop(o, options, "devStatus", || json!(n.dev_status.id()));
    }
    if n.dev_changed {
        insert_prop(o, options, "devChanged", || json!(true));
    }
    if !n.dev_note.is_empty() {
        insert_prop(o, options, "devNote", || json!(n.dev_note));
    }
    if let NodeKind::Section { contents_hidden } = n.kind {
        insert_prop(o, options, "sectionContentsHidden", || json!(contents_hidden));
    }
    if n.is_mask {
        insert_prop(o, options, "isMask", || json!(true));
        insert_prop(o, options, "maskType", || json!(n.mask_type.id()));
    }
    if !n.grids.is_empty() {
        insert_prop(o, options, "grids", || camel(serde_json::to_value(&n.grids).unwrap_or(Value::Null)));
    }
    if !n.exports.is_empty() {
        insert_prop(o, options, "exports", || serde_json::to_value(&n.exports).unwrap_or(Value::Null));
    }
    if let Some(b) = &n.board {
        insert_prop(o, options, "board", || serde_json::to_value(b).unwrap_or(Value::Null));
    }
    if !n.annotations.is_empty() {
        insert_prop(o, options, "annotations", || serde_json::to_value(&n.annotations).unwrap_or(Value::Null));
    }
    if !n.code_links.is_empty() {
        insert_prop(o, options, "codeLinks", || camel(serde_json::to_value(&n.code_links).unwrap_or(Value::Null)));
    }
    if !n.measurements.is_empty() {
        insert_prop(o, options, "measurements", || serde_json::to_value(&n.measurements).unwrap_or(Value::Null));
    }
    if options.includes("devResources") {
        let dev = crate::dev_resources::list_json(doc, n.id);
        if dev.as_array().is_some_and(|a| !a.is_empty()) {
            insert_prop(o, options, "devResources", || dev);
        }
    }
    if !n.interactions.is_empty() {
        insert_prop(o, options, "interactions", || serde_json::to_value(&n.interactions).unwrap_or(Value::Null));
    }
    if !n.styles.is_empty() {
        insert_prop(o, options, "styles", || serde_json::to_value(&n.styles).unwrap_or(Value::Null));
    }
    if !n.bound.is_empty() {
        insert_prop(o, options, "boundVariables", || json!(n.bound));
    }
    if !n.overrides.is_empty() {
        insert_prop(o, options, "overrides", || json!(n.overrides));
    }
    if let Some(slot) = options.includes("slot").then(|| crate::components::slot_name(doc, n.id)).flatten() {
        insert_prop(o, options, "slot", || json!(slot));
    }
    // An operand combining by an operation of its own.
    if let Some(op) = n.bool_op.filter(|_| options.includes("operandOperation") && crate::booleans::group_of(doc, n.id).is_some()) {
        insert_prop(o, options, "operandOperation", || json!(op));
    }
}

fn text_props(t: &TextData, o: &mut serde_json::Map<String, Value>, options: &InspectOptions) {
    insert_prop(o, options, "characters", || {
        if options.outline {
            let mut chars = t.characters.chars();
            let mut text: String = chars.by_ref().take(120).collect();
            if chars.next().is_some() {
                text.push_str("...");
            }
            json!(text)
        } else {
            json!(t.characters)
        }
    });
    insert_prop(o, options, "fontFamily", || json!(t.style.font_family));
    insert_prop(o, options, "fontStyle", || json!(t.style.font_style));
    insert_prop(o, options, "fontSize", || json!(t.style.font_size));
    if !t.style.variations.is_empty() {
        insert_prop(o, options, "fontVariations", || json!(t.style.variations));
    }
    insert_prop(o, options, "lineHeight", || json!(t.style.line_height.label()));
    insert_prop(o, options, "letterSpacing", || json!(t.style.letter_spacing.label()));
    insert_prop(o, options, "textAlign", || json!(t.align_h));
    insert_prop(o, options, "textAutoResize", || json!(t.auto_resize));
    if t.max_lines > 0 {
        insert_prop(o, options, "maxLines", || json!(t.max_lines));
    }
    if let Some(tp) = &t.path {
        insert_prop(o, options, "textPath", || json!(tp));
    }
    if t.style.decoration != TextDecoration::None {
        insert_prop(o, options, "textDecoration", || json!(t.style.decoration));
    }
    if let Some(l) = t.style.decoration_look.as_deref() {
        insert_prop(o, options, "textDecorationStyle", || json!(l.style));
        insert_prop(o, options, "textDecorationThickness", || json!(l.thickness.label()));
        insert_prop(o, options, "textDecorationOffset", || json!(l.offset.label()));
        insert_prop(o, options, "textDecorationSkipInk", || json!(l.skip_ink));
        insert_prop(o, options, "textDecorationColor", || l.color.map_or(json!("auto"), |c| json!(format!("#{}", c.to_hex()))));
    }
    // Lists: one style for the layer, or each paragraph's type, nesting level and start.
    if t.paragraphs.is_empty() {
        if t.list != figcraft_doc::ListStyle::None {
            insert_prop(o, options, "listStyle", || json!(t.list));
        }
    } else {
        insert_prop(o, options, "lineTypes", || t.paragraphs.iter().map(|p| json!(p.list)).collect());
        insert_prop(o, options, "lineIndentations", || t.paragraphs.iter().map(|p| json!(p.indent)).collect());
        if t.paragraphs.iter().any(|p| p.start_offset > 0) {
            insert_prop(o, options, "listStartOffsets", || t.paragraphs.iter().map(|p| json!(p.start_offset)).collect());
        }
    }
    if t.list_spacing != 0.0 {
        insert_prop(o, options, "listSpacing", || json!(t.list_spacing));
    }
    for (k, on) in [("hangingPunctuation", t.hanging_punctuation), ("hangingList", t.hanging_list)] {
        if on {
            insert_prop(o, options, k, || json!(true));
        }
    }
    if t.leading_trim != figcraft_doc::LeadingTrim::None {
        insert_prop(o, options, "leadingTrim", || json!(t.leading_trim));
    }
    if !t.runs.is_empty() {
        insert_prop(o, options, "styledRuns", || serde_json::to_value(&t.runs).unwrap_or(Value::Null));
    }
    // Chars linked to text styles: [start, end, style id] (the layer as a whole: `styles.text`).
    if options.includes("textStyleRanges") {
        let linked: Vec<Value> = t.spans().into_iter().filter_map(|(a, b, s)| s.style_id.as_ref().map(|id| json!([a, b, id]))).collect();
        if !linked.is_empty() {
            insert_prop(o, options, "textStyleRanges", || Value::Array(linked));
        }
    }
}

fn kind_props(doc: &Document, n: &Node, o: &mut serde_json::Map<String, Value>, options: &InspectOptions) {
    match &n.kind {
        NodeKind::Ellipse { start, sweep, inner } if (*sweep).abs() < std::f64::consts::TAU - 1e-9 || *inner > 0.0 => {
            insert_prop(o, options, "arc", || json!({"start": start.to_degrees(), "sweep": sweep.to_degrees(), "inner": inner}));
        }
        NodeKind::Polygon { count } => {
            insert_prop(o, options, "pointCount", || json!(count));
        }
        NodeKind::Star { count, inner_ratio } => {
            insert_prop(o, options, "pointCount", || json!(count));
            insert_prop(o, options, "innerRadius", || json!(inner_ratio));
        }
        NodeKind::BooleanOp { op } => {
            insert_prop(o, options, "booleanOperation", || json!(op));
        }
        NodeKind::Instance { component, props } => {
            insert_prop(o, options, "mainComponent", || json!(component));
            insert_prop(o, options, "mainComponentName", || json!(doc.node(*component).map(|c| c.name.clone())));
            if !props.is_empty() {
                insert_prop(o, options, "componentProperties", || json!(props));
            }
        }
        NodeKind::Component { description, props, .. } => {
            if !description.is_empty() {
                insert_prop(o, options, "description", || json!(description));
            }
            if !props.is_empty() {
                insert_prop(o, options, "componentPropertyDefinitions", || serde_json::to_value(props).unwrap_or(Value::Null));
            }
        }
        NodeKind::Vector { network } => {
            insert_prop(o, options, "vertices", || json!(network.vertices.len()));
            insert_prop(o, options, "segments", || json!(network.segments.len()));
        }
        NodeKind::Page { background, flows, .. } => {
            insert_prop(o, options, "background", || json!(background.to_css_hex()));
            if !flows.is_empty() {
                insert_prop(o, options, "flows", || serde_json::to_value(flows).unwrap_or(Value::Null));
            }
        }
        _ => {}
    }
}

fn round(v: f64) -> f64 {
    figcraft_geom::round_to(v, 2)
}

fn paints(p: &[figcraft_color::Paint]) -> Value {
    Value::Array(
        p.iter()
            .map(|x| match &x.kind {
                figcraft_color::PaintKind::Solid { color } => {
                    let mut v = json!({"type": "solid", "color": color.to_css_hex()});
                    if x.opacity < 1.0 {
                        v["opacity"] = json!(x.opacity);
                    }
                    if x.blend != figcraft_color::BlendMode::Normal {
                        v["blend"] = json!(x.blend);
                    }
                    if !x.visible {
                        v["visible"] = json!(false);
                    }
                    v
                }
                _ => serde_json::to_value(x).unwrap_or(Value::Null),
            })
            .collect(),
    )
}

/// The whole document (current page in depth, other pages summarised).
pub fn document(s: &Session, depth: usize, all_pages: bool) -> Value {
    document_with_options(s, depth, all_pages, &InspectOptions::default())
}

/// The document metadata and a tree built with only the selected layer properties.
pub fn document_with_options(s: &Session, depth: usize, all_pages: bool, options: &InspectOptions) -> Value {
    let Ok(ds) = s.ds() else { return json!({"documents": 0}) };
    let doc = &ds.doc;
    let pages: Vec<Value> =
        doc.pages
            .iter()
            .map(|p| {
                if all_pages || *p == ds.page { node_json_with_options(doc, *p, depth, options) } else { node_json_with_options(doc, *p, 0, options) }
            })
            .collect();
    json!({
        "id": ds.uid,
        "name": doc.name,
        "kind": doc.kind.id(),
        "path": ds.path,
        "dirty": ds.is_dirty(),
        "currentPage": ds.page,
        "selection": ds.selection,
        "pages": pages,
        "styles": doc.styles.iter().map(|st| json!({"id": st.id, "name": st.name, "type": st.kind.type_name()})).collect::<Vec<_>>(),
        "variables": doc.variables.variables.len(),
        "images": doc.images.len(),
        "colorProfile": doc.color_profile.id(),
        "textEdit": ds.text_edit,
        "history": {"undo": s.undo_labels(20), "redo": s.undo_depth().1},
    })
}

/// snake_case object keys → camelCase (inspect output matches the keys `node.set` accepts).
pub fn camel(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .map(|(k, v)| {
                    let mut out = String::new();
                    let mut up = false;
                    for c in k.chars() {
                        if c == '_' {
                            up = true;
                        } else if up {
                            out.extend(c.to_uppercase());
                            up = false;
                        } else {
                            out.push(c);
                        }
                    }
                    (out, camel(v))
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.into_iter().map(camel).collect()),
        other => other,
    }
}
